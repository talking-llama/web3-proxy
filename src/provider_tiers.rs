///! Communicate with groups of web3 providers
use arc_swap::ArcSwap;
use governor::clock::{QuantaClock, QuantaInstant};
use governor::NotUntil;
use std::cmp;
use std::collections::HashMap;
use std::fmt;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::block_watcher::{BlockWatcher, SyncStatus};
use crate::provider::Web3Connection;

// TODO: move the rate limiter into the connection
pub type Web3ConnectionMap = HashMap<String, Web3Connection>;

/// Load balance to the rpc
pub struct Web3ProviderTier {
    /// TODO: what type for the rpc? Vec<String> isn't great. i think we want this to be the key for the provider and not the provider itself
    /// TODO: we probably want a better lock
    synced_rpcs: ArcSwap<Vec<String>>,
    rpcs: Vec<String>,
    connections: Arc<Web3ConnectionMap>,
}

impl fmt::Debug for Web3ProviderTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: the default formatter takes forever to write. this is too quiet though
        write!(f, "Web3ProviderTier")
    }
}

impl Web3ProviderTier {
    pub async fn try_new(
        servers: Vec<(&str, u32)>,
        http_client: Option<reqwest::Client>,
        block_watcher: Arc<BlockWatcher>,
        clock: &QuantaClock,
    ) -> anyhow::Result<Web3ProviderTier> {
        let mut rpcs: Vec<String> = vec![];
        let mut connections = HashMap::new();

        for (s, limit) in servers.into_iter() {
            rpcs.push(s.to_string());

            let ratelimiter = if limit > 0 {
                let quota = governor::Quota::per_second(NonZeroU32::new(limit).unwrap());

                let rate_limiter = governor::RateLimiter::direct_with_clock(quota, clock);

                Some(rate_limiter)
            } else {
                None
            };

            let connection = Web3Connection::try_new(
                s.to_string(),
                http_client.clone(),
                block_watcher.clone_sender(),
                ratelimiter,
            )
            .await?;

            connections.insert(s.to_string(), connection);
        }

        Ok(Web3ProviderTier {
            synced_rpcs: ArcSwap::from(Arc::new(vec![])),
            rpcs,
            connections: Arc::new(connections),
        })
    }

    pub fn clone_connections(&self) -> Arc<Web3ConnectionMap> {
        self.connections.clone()
    }

    pub fn clone_rpcs(&self) -> Vec<String> {
        self.rpcs.clone()
    }

    pub fn update_synced_rpcs(
        &self,
        block_watcher: Arc<BlockWatcher>,
        allowed_lag: u64,
    ) -> anyhow::Result<()> {
        let mut available_rpcs = self.rpcs.clone();

        // collect sync status for all the rpcs
        let sync_status: HashMap<String, SyncStatus> = available_rpcs
            .clone()
            .into_iter()
            .map(|rpc| {
                let status = block_watcher.sync_status(&rpc, allowed_lag);
                (rpc, status)
            })
            .collect();

        // sort rpcs by their sync status and active connections
        available_rpcs.sort_unstable_by(|a, b| {
            let a_synced = sync_status.get(a).unwrap();
            let b_synced = sync_status.get(b).unwrap();

            match (a_synced, b_synced) {
                (SyncStatus::Synced(a), SyncStatus::Synced(b)) => {
                    if a != b {
                        return a.cmp(b);
                    }
                    // else they are equal and we want to compare on active connections
                }
                (SyncStatus::Synced(_), SyncStatus::Unknown) => {
                    return cmp::Ordering::Greater;
                }
                (SyncStatus::Unknown, SyncStatus::Synced(_)) => {
                    return cmp::Ordering::Less;
                }
                (SyncStatus::Unknown, SyncStatus::Unknown) => {
                    // neither rpc is synced
                    // this means neither will have connections
                    return cmp::Ordering::Equal;
                }
                (SyncStatus::Synced(_), SyncStatus::Behind(_)) => {
                    return cmp::Ordering::Greater;
                }
                (SyncStatus::Behind(_), SyncStatus::Synced(_)) => {
                    return cmp::Ordering::Less;
                }
                (SyncStatus::Behind(_), SyncStatus::Unknown) => {
                    return cmp::Ordering::Greater;
                }
                (SyncStatus::Behind(a), SyncStatus::Behind(b)) => {
                    if a != b {
                        return a.cmp(b);
                    }
                    // else they are equal and we want to compare on active connections
                }
                (SyncStatus::Unknown, SyncStatus::Behind(_)) => {
                    return cmp::Ordering::Less;
                }
            }

            // sort on active connections
            self.connections
                .get(a)
                .unwrap()
                .cmp(self.connections.get(b).unwrap())
        });

        // filter out
        let synced_rpcs: Vec<String> = available_rpcs
            .into_iter()
            .take_while(|rpc| matches!(sync_status.get(rpc).unwrap(), SyncStatus::Synced(_)))
            .collect();

        self.synced_rpcs.swap(Arc::new(synced_rpcs));

        Ok(())
    }

    /// get the best available rpc server
    pub async fn next_upstream_server(&self) -> Result<String, Option<NotUntil<QuantaInstant>>> {
        let mut earliest_not_until = None;

        for selected_rpc in self.synced_rpcs.load().iter() {
            // increment our connection counter
            if let Err(not_until) = self
                .connections
                .get(selected_rpc)
                .unwrap()
                .try_inc_active_requests()
            {
                // TODO: do this better
                if earliest_not_until.is_none() {
                    earliest_not_until = Some(not_until);
                } else {
                    let earliest_possible =
                        earliest_not_until.as_ref().unwrap().earliest_possible();
                    let new_earliest_possible = not_until.earliest_possible();

                    if earliest_possible > new_earliest_possible {
                        earliest_not_until = Some(not_until);
                    }
                }
                continue;
            }

            // return the selected RPC
            return Ok(selected_rpc.clone());
        }

        // this might be None
        Err(earliest_not_until)
    }

    /// get all available rpc servers
    pub async fn get_upstream_servers(
        &self,
    ) -> Result<Vec<String>, Option<NotUntil<QuantaInstant>>> {
        let mut earliest_not_until = None;
        let mut selected_rpcs = vec![];
        for selected_rpc in self.synced_rpcs.load().iter() {
            // check rate limits and increment our connection counter
            // TODO: share code with next_upstream_server
            if let Err(not_until) = self
                .connections
                .get(selected_rpc)
                .unwrap()
                .try_inc_active_requests()
            {
                if earliest_not_until.is_none() {
                    earliest_not_until = Some(not_until);
                } else {
                    let earliest_possible =
                        earliest_not_until.as_ref().unwrap().earliest_possible();
                    let new_earliest_possible = not_until.earliest_possible();

                    if earliest_possible > new_earliest_possible {
                        earliest_not_until = Some(not_until);
                    }
                }
                continue;
            }

            // this is rpc should work
            selected_rpcs.push(selected_rpc.clone());
        }

        if !selected_rpcs.is_empty() {
            return Ok(selected_rpcs);
        }

        // return the earliest not_until (if no rpcs are synced, this will be None)
        Err(earliest_not_until)
    }
}