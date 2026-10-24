//! GitHub sync for petaricon projects, over the GitHub REST (Git Data) API only.
//!
//! No Slint, no git binary. `api` holds the `GitHubApi` trait and the ureq/rustls
//! implementation, `state` the `.sync` file and local file helpers, `sync` the procedures.

pub mod api;
#[cfg(feature = "test-support")]
pub mod fake;
pub mod state;
pub mod sync;

pub use api::{ApiError, GitHubApi, HttpApi, NewEntry, TreeEntry, UpdateError};
pub use state::{FileEntry, SyncState, git_blob_sha, is_managed};
pub use sync::{
    Change, ChangeKind, PullReport, PushReport, SyncError, clone_into, keep_local, local_changes,
    pull, push, take_remote,
};
