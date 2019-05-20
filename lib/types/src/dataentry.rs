// Copyright Facebook, Inc. 2019.

use bytes::Bytes;
use crypto::{digest::Digest, sha1::Sha1};
use failure::{ensure, Fallible};
use serde_derive::{Deserialize, Serialize};

use crate::{key::Key, node::Node, parents::Parents};

/// Structure representing source control data (typically
/// either file content or a tree entry) on the wire.
/// Includes the information required to add the data to
/// a MutableDataPack, along with the node's parent
/// information to allow for hash verification.
#[derive(
    Clone,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize
)]
pub struct DataEntry {
    key: Key,
    data: Bytes,
    parents: Parents,
}

impl DataEntry {
    pub fn new(key: Key, data: Bytes, parents: Parents) -> Self {
        Self { key, data, parents }
    }

    pub fn key(&self) -> &Key {
        &self.key
    }

    /// Get this entry's data content. If validate is set to true, this method
    /// will recompute the entry's node hash and verify that it matches the
    /// expected node hash in the entry's key, returning an error otherwise.
    pub fn data(&self, validate: bool) -> Fallible<Bytes> {
        if validate {
            self.validate()?;
        }
        Ok(self.data.clone())
    }

    /// Compute the filenode hash of this `DataEntry` using its parents and
    /// content, and compare it with the known node hash from the entry's `Key`.
    fn validate(&self) -> Fallible<()> {
        // Mercurial hashes the parent nodes in sorted order
        // when computing the node hash.
        let (p1, p2) = match self.parents.clone().into_nodes() {
            (p1, p2) if p1 > p2 => (p2, p1),
            (p1, p2) => (p1, p2),
        };

        let mut hash = [0u8; 20];
        let mut hasher = Sha1::new();
        hasher.input(p1.as_ref());
        hasher.input(p2.as_ref());
        hasher.input(&self.data);
        hasher.result(&mut hash);

        let computed = Node::from_byte_array(hash);
        let expected = &self.key.node;

        ensure!(
            &computed == expected,
            "Content hash validation failed. Expected: {}; Computed: {}",
            expected.to_hex(),
            computed.to_hex()
        );

        Ok(())
    }
}
