// Copyright (c) 2004-present, Facebook, Inc.
// All Rights Reserved.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2 or any later version.

//! Root manifest, tree nodes

use std::collections::BTreeMap;
use std::str;

use failure_ext::{bail_msg, ensure_msg, Error, FutureFailureErrorExt, Result, ResultExt};
use futures::future::{Future, IntoFuture};
use futures_ext::{BoxFuture, FutureExt};

use context::CoreContext;
use mercurial_types::nodehash::{HgNodeHash, NULL_HASH};
use mercurial_types::{
    FileType, HgBlob, HgEntry, HgEntryId, HgFileNodeId, HgManifest, HgManifestEnvelope,
    HgManifestId, MPathElement, Type,
};

use blobstore::Blobstore;

use crate::errors::*;
use crate::file::HgBlobEntry;
use repo_blobstore::RepoBlobstore;

#[derive(Debug, Eq, PartialEq)]
pub struct ManifestContent {
    pub files: BTreeMap<MPathElement, HgEntryId>,
}

impl ManifestContent {
    pub fn new_empty() -> Self {
        Self {
            files: BTreeMap::new(),
        }
    }

    // Each manifest revision contains a list of the file revisions in each changeset, in the form:
    //
    // <filename>\0<hex file revision id>[<flags>]\n
    //
    // Source: mercurial/parsers.c:parse_manifest()
    //
    // NB: filenames are sequences of non-zero bytes, not strings
    fn parse_impl(data: &[u8]) -> Result<BTreeMap<MPathElement, HgEntryId>> {
        let mut files = BTreeMap::new();

        for line in data.split(|b| *b == b'\n') {
            if line.len() == 0 {
                break;
            }

            let (name, rest) = match find(line, &0) {
                None => bail_msg!("Malformed entry: no \\0"),
                Some(nil) => {
                    let (name, rest) = line.split_at(nil);
                    if let Some((_, hash)) = rest.split_first() {
                        (name, hash)
                    } else {
                        bail_msg!("Malformed entry: no hash");
                    }
                }
            };

            let path = MPathElement::new(name.to_vec()).context("invalid path in manifest")?;
            let entry_id = parse_hg_entry(rest)?;

            files.insert(path, entry_id);
        }

        Ok(files)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        Ok(Self {
            files: Self::parse_impl(data)?,
        })
    }
}

pub fn fetch_raw_manifest_bytes(
    ctx: CoreContext,
    blobstore: &RepoBlobstore,
    manifest_id: HgManifestId,
) -> BoxFuture<HgBlob, Error> {
    fetch_manifest_envelope(ctx, blobstore, manifest_id)
        .map(move |envelope| {
            let envelope = envelope.into_mut();
            HgBlob::from(envelope.contents)
        })
        .from_err()
        .boxify()
}

pub fn fetch_manifest_envelope(
    ctx: CoreContext,
    blobstore: &RepoBlobstore,
    manifest_id: HgManifestId,
) -> impl Future<Item = HgManifestEnvelope, Error = Error> {
    fetch_manifest_envelope_opt(ctx, blobstore, manifest_id)
        .and_then(move |envelope| {
            let envelope = envelope.ok_or(ErrorKind::HgContentMissing(
                manifest_id.into_nodehash(),
                Type::Tree,
            ))?;
            Ok(envelope)
        })
        .from_err()
}

/// Like `fetch_manifest_envelope`, but returns None if the manifest wasn't found.
pub fn fetch_manifest_envelope_opt(
    ctx: CoreContext,
    blobstore: &RepoBlobstore,
    node_id: HgManifestId,
) -> impl Future<Item = Option<HgManifestEnvelope>, Error = Error> {
    let blobstore_key = node_id.blobstore_key();
    blobstore
        .get(ctx, blobstore_key.clone())
        .context("While fetching manifest envelope blob")
        .map_err(Error::from)
        .and_then(move |bytes| {
            let blobstore_bytes = match bytes {
                Some(bytes) => bytes,
                None => return Ok(None),
            };
            let envelope = HgManifestEnvelope::from_blob(blobstore_bytes.into())?;
            if node_id.into_nodehash() != envelope.node_id() {
                bail_msg!(
                    "Manifest ID mismatch (requested: {}, got: {})",
                    node_id,
                    envelope.node_id()
                );
            }
            Ok(Some(envelope))
        })
        .with_context(|_| ErrorKind::ManifestDeserializeFailed(blobstore_key))
        .from_err()
}

pub struct BlobManifest {
    blobstore: RepoBlobstore,
    node_id: HgNodeHash,
    p1: Option<HgNodeHash>,
    p2: Option<HgNodeHash>,
    // See the documentation in mercurial_types/if/mercurial.thrift for why this exists.
    computed_node_id: HgNodeHash,
    content: ManifestContent,
}

impl BlobManifest {
    pub fn load(
        ctx: CoreContext,
        blobstore: &RepoBlobstore,
        manifestid: HgManifestId,
    ) -> BoxFuture<Option<Self>, Error> {
        if manifestid.clone().into_nodehash() == NULL_HASH {
            Ok(Some(BlobManifest {
                blobstore: blobstore.clone(),
                node_id: NULL_HASH,
                p1: None,
                p2: None,
                computed_node_id: NULL_HASH,
                content: ManifestContent::new_empty(),
            }))
            .into_future()
            .boxify()
        } else {
            fetch_manifest_envelope_opt(ctx, &blobstore, manifestid)
                .and_then({
                    let blobstore = blobstore.clone();
                    move |envelope| match envelope {
                        Some(envelope) => Ok(Some(Self::parse(blobstore, envelope)?)),
                        None => Ok(None),
                    }
                })
                .context(format!(
                    "When loading manifest {} from blobstore",
                    manifestid
                ))
                .from_err()
                .boxify()
        }
    }

    pub fn parse(blobstore: RepoBlobstore, envelope: HgManifestEnvelope) -> Result<Self> {
        let envelope = envelope.into_mut();
        let content = ManifestContent::parse(envelope.contents.as_ref()).with_context(|_| {
            format!(
                "while parsing contents for manifest ID {}",
                envelope.node_id
            )
        })?;
        Ok(BlobManifest {
            blobstore,
            node_id: envelope.node_id,
            p1: envelope.p1,
            p2: envelope.p2,
            computed_node_id: envelope.computed_node_id,
            content,
        })
    }

    #[inline]
    pub fn node_id(&self) -> HgNodeHash {
        self.node_id
    }

    #[inline]
    pub fn p1(&self) -> Option<HgNodeHash> {
        self.p1
    }

    #[inline]
    pub fn p2(&self) -> Option<HgNodeHash> {
        self.p2
    }

    #[inline]
    pub fn computed_node_id(&self) -> HgNodeHash {
        self.computed_node_id
    }
}

impl HgManifest for BlobManifest {
    fn lookup(&self, path: &MPathElement) -> Option<Box<dyn HgEntry + Sync>> {
        self.content.files.get(path).map({
            move |entry_id| {
                HgBlobEntry::new(
                    self.blobstore.clone(),
                    path.clone(),
                    entry_id.clone().into_nodehash(),
                    entry_id.get_type(),
                )
                .boxed()
            }
        })
    }

    fn list(&self) -> Box<dyn Iterator<Item = Box<dyn HgEntry + Sync>> + Send> {
        let list_iter = self.content.files.clone().into_iter().map({
            let blobstore = self.blobstore.clone();
            move |(path, entry_id)| {
                HgBlobEntry::new(
                    blobstore.clone(),
                    path,
                    entry_id.clone().into_nodehash(),
                    entry_id.get_type(),
                )
                .boxed()
            }
        });
        Box::new(list_iter)
    }
}

fn parse_hg_entry(data: &[u8]) -> Result<HgEntryId> {
    ensure_msg!(data.len() >= 40, "hash too small: {:?}", data);

    let (hash, flags) = data.split_at(40);
    let hash = str::from_utf8(hash)
        .map_err(|err| Error::from(err))
        .and_then(|hash| hash.parse::<HgNodeHash>())
        .with_context(|_| format!("malformed hash: {:?}", hash))?;
    ensure_msg!(flags.len() <= 1, "More than 1 flag: {:?}", flags);

    let hg_entry_id = if flags.len() == 0 {
        HgEntryId::File(FileType::Regular, HgFileNodeId::new(hash))
    } else {
        match flags[0] {
            b'l' => HgEntryId::File(FileType::Symlink, HgFileNodeId::new(hash)),
            b'x' => HgEntryId::File(FileType::Executable, HgFileNodeId::new(hash)),
            b't' => HgEntryId::Manifest(HgManifestId::new(hash)),
            unk => bail_msg!("Unknown flag {}", unk),
        }
    };

    Ok(hg_entry_id)
}

fn find<T>(haystack: &[T], needle: &T) -> Option<usize>
where
    T: PartialEq,
{
    haystack.iter().position(|e| e == needle)
}
