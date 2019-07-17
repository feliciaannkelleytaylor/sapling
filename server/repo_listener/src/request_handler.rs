// Copyright (c) 2004-present, Facebook, Inc.
// All Rights Reserved.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2 or any later version.

use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::failure::{prelude::*, SlogKVError};
use configerator::ConfigeratorAPI;
use fbwhoami::FbWhoAmI;
use futures::{Future, Sink, Stream};
use futures_stats::Timed;
use limits::types::{MononokeThrottleLimit, MononokeThrottleLimits};
use serde_json;
use slog::{self, Drain, Level, Logger};
use slog_ext::SimpleFormatWithError;
use slog_kvfilter::KVFilter;
use slog_term;
use stats::Histogram;
use time_ext::DurationExt;
use tracing::{TraceContext, Traced};
use uuid::Uuid;

use hgproto::{sshproto, HgProtoHandler};
use repo_client::RepoClient;
use scuba_ext::ScubaSampleBuilderExt;
use sshrelay::{SenderBytesWrite, SshEnvVars, Stdio};

use crate::repo_handlers::RepoHandler;

use context::{CoreContext, Metric};
use hooks::HookManager;

lazy_static! {
    static ref DATACENTER_REGION_PREFIX: String = {
        FbWhoAmI::new()
            .expect("failed to init fbwhoami")
            .get_region_data_center_prefix()
            .expect("failed to get region from fbwhoami")
            .to_string()
    };
}

// It's made public so that the code that creates ConfigeratorAPI can subscribe to this category
pub const CONFIGERATOR_LIMITS_CONFIG: &str = "scm/mononoke/loadshedding/limits";
const CONFIGERATOR_TIMEOUT_MS: usize = 25;
const DEFAULT_PERCENTAGE: f64 = 100.0;

define_stats! {
    prefix = "mononoke.request_handler";
    wireproto_ms:
        histogram(500, 0, 100_000, AVG, SUM, COUNT; P 5; P 25; P 50; P 75; P 95; P 97; P 99),
}

pub fn request_handler(
    RepoHandler {
        logger,
        scuba,
        wireproto_scribe_category,
        repo,
        hash_validation_percentage,
        lca_hint,
        phases_hint,
        preserve_raw_bundle2,
        pure_push_allowed,
        support_bundle2_listkeys,
    }: RepoHandler,
    stdio: Stdio,
    hook_manager: Arc<HookManager>,
    load_limiting_config: Option<(Arc<ConfigeratorAPI>, String)>,
) -> impl Future<Item = (), Error = ()> {
    let mut scuba_logger = scuba;
    let Stdio {
        stdin,
        stdout,
        stderr,
        mut preamble,
    } = stdio;

    let session_uuid = match preamble
        .misc
        .get("session_uuid")
        .and_then(|v| Uuid::parse_str(v).ok())
    {
        Some(session_uuid) => session_uuid,
        None => {
            let session_uuid = Uuid::new_v4();
            preamble
                .misc
                .insert("session_uuid".to_owned(), format!("{}", session_uuid));
            session_uuid
        }
    };

    // Info per wireproto command within this session
    let wireproto_calls = Arc::new(Mutex::new(Vec::new()));
    let trace = TraceContext::new(session_uuid, Instant::now());

    // Per-connection logging drain that forks output to normal log and back to client stderr
    let conn_log = {
        let stderr_write = SenderBytesWrite {
            chan: stderr.wait(),
        };
        let client_drain = slog_term::PlainSyncDecorator::new(stderr_write);
        let client_drain = SimpleFormatWithError::new(client_drain);
        let client_drain = KVFilter::new(client_drain, Level::Critical).only_pass_any_on_all_keys(
            (hashmap! {
                "remote".into() => hashset!["true".into(), "remote_only".into()],
            })
            .into(),
        );

        let server_drain = KVFilter::new(logger, Level::Critical).always_suppress_any(
            (hashmap! {
                "remote".into() => hashset!["remote_only".into()],
            })
            .into(),
        );

        // Don't fail logging if the client goes away
        let drain = slog::Duplicate::new(client_drain, server_drain).ignore_res();
        Logger::root(drain, o!("session_uuid" => format!("{}", session_uuid)))
    };

    scuba_logger.log_with_msg("Connection established", None);
    let client_hostname = preamble
        .misc
        .get("source_hostname")
        .cloned()
        .unwrap_or("".to_string());

    let load_limiting_config = match load_limiting_config {
        Some((configerator_api, category)) => {
            loadlimiting_configs(configerator_api, client_hostname).map(|limits| (limits, category))
        }
        None => None,
    };

    let ctx = CoreContext::new(
        session_uuid,
        conn_log,
        scuba_logger.clone(),
        wireproto_scribe_category,
        trace.clone(),
        preamble.misc.get("unix_username").cloned(),
        SshEnvVars::from_map(&preamble.misc),
        load_limiting_config,
    );

    // Construct a hg protocol handler
    let proto_handler = HgProtoHandler::new(
        ctx.clone(),
        stdin,
        RepoClient::new(
            repo.clone(),
            ctx.clone(),
            hash_validation_percentage,
            lca_hint,
            phases_hint,
            preserve_raw_bundle2,
            pure_push_allowed,
            hook_manager,
            support_bundle2_listkeys,
        ),
        sshproto::HgSshCommandDecode,
        sshproto::HgSshCommandEncode,
        wireproto_calls.clone(),
    );

    // send responses back
    let endres = proto_handler
        .inspect({
            cloned!(ctx);
            move |bytes| ctx.bump_load(Metric::EgressBytes, bytes.len() as f64)
        })
        .map_err(Error::from)
        .forward(stdout)
        .map(|_| ());

    // If we got an error at this point, then catch it and print a message
    endres
        .traced(&trace, "wireproto request", trace_args!())
        .timed(move |stats, result| {
            let mut wireproto_calls = wireproto_calls.lock().expect("lock poisoned");
            let wireproto_calls = mem::replace(&mut *wireproto_calls, Vec::new());

            STATS::wireproto_ms.add_value(stats.completion_time.as_millis_unchecked() as i64);
            scuba_logger
                .add_future_stats(&stats)
                .add("wireproto_commands", wireproto_calls);

            match result {
                Ok(_) => scuba_logger.log_with_msg("Request finished - Success", None),
                Err(err) => {
                    scuba_logger.log_with_msg("Request finished - Failure", format!("{:#?}", err));
                }
            }
            scuba_logger.log_with_trace(&trace)
        })
        .map_err(move |err| {
            error!(ctx.logger(), "Command failed";
                SlogKVError(err),
                "remote" => "true"
            );
        })
}

fn loadlimiting_configs(
    configerator_api: Arc<ConfigeratorAPI>,
    client_hostname: String,
) -> Option<MononokeThrottleLimit> {
    let data = configerator_api
        .get_entity(CONFIGERATOR_LIMITS_CONFIG, CONFIGERATOR_TIMEOUT_MS)
        .ok();
    data.and_then(|data| {
        let config: Option<MononokeThrottleLimits> = serde_json::from_str(&data.contents).ok();
        config
    })
    .and_then(|config| {
        let region_percentage = config
            .datacenter_prefix_capacity
            .get(&*DATACENTER_REGION_PREFIX)
            .copied()
            .unwrap_or(DEFAULT_PERCENTAGE);
        let host_scheme = hostname_scheme(client_hostname);
        let limit = config
            .hostprefixes
            .get(&host_scheme)
            .or(Some(&config.defaults))
            .copied();

        match limit {
            Some(limit) => Some(MononokeThrottleLimit {
                egress_bytes: limit.egress_bytes * region_percentage / 100.0,
                ingress_blobstore_bytes: limit.ingress_blobstore_bytes * region_percentage / 100.0,
                total_manifests: limit.total_manifests * region_percentage / 100.0,
                quicksand_manifests: limit.quicksand_manifests * region_percentage / 100.0,
            }),
            _ => None,
        }
    })
}

/// Translates a hostname in to a host scheme:
///   devvm001.lla1.facebook.com -> devvm
///   hg001.lla1.facebook.com -> hg
fn hostname_scheme(hostname: String) -> String {
    let mut hostprefix = hostname.clone();
    let index = hostprefix.find(|c: char| !c.is_ascii_alphabetic());
    match index {
        Some(index) => hostprefix.truncate(index),
        None => {}
    }
    hostprefix
}
