//! Who may read a thread, as a SQL condition, for listings that must filter
//! in the query rather than after it.
//!
//! This is the query form of `maidan_auth::authorize_thread`, and the two
//! must agree: the thread's channel is in the caller's workspace, and then a
//! `__dm__` thread needs the caller in its DM or group DM, a private channel
//! needs a `channel_members` row, and any other channel is open. Filtering a
//! page after the query costs a batch per hidden row and makes page size
//! depend on what the caller cannot see. `tests/thread_access.rs` checks the
//! two against each other on both backends.

/// A condition true when `member` may read the thread `thread` names.
/// `thread` is a SQL expression; `workspace`, `member` and `dm_channel` are
/// placeholders (with any cast the backend needs) bound to the caller's
/// workspace, its member id and [`maidan_types::DM_CHANNEL_NAME`]. The same
/// SQL runs on Postgres and SQLite.
pub(crate) fn readable_thread(
    thread: &str,
    workspace: &str,
    member: &str,
    dm_channel: &str,
) -> String {
    format!(
        "EXISTS (SELECT 1 FROM maidan_threads rt_thread
                 JOIN maidan_channels rt_channel ON rt_channel.id = rt_thread.channel_id
                 WHERE rt_thread.id = {thread} AND rt_channel.workspace_id = {workspace}
                   AND CASE
                         WHEN rt_channel.name = {dm_channel} THEN
                              EXISTS (SELECT 1 FROM maidan_dm_conversations rt_dm
                                      WHERE rt_dm.thread_id = rt_thread.id
                                        AND {member} IN (rt_dm.member_low_id, rt_dm.member_high_id))
                           OR EXISTS (SELECT 1 FROM maidan_group_dm_conversations rt_group
                                      JOIN maidan_group_dm_members rt_group_member
                                           ON rt_group_member.group_dm_id = rt_group.id
                                      WHERE rt_group.thread_id = rt_thread.id
                                        AND rt_group_member.member_id = {member})
                         WHEN rt_channel.private THEN
                              EXISTS (SELECT 1 FROM maidan_channel_members rt_member
                                      WHERE rt_member.channel_id = rt_channel.id
                                        AND rt_member.member_id = {member})
                         ELSE TRUE
                       END)"
    )
}

/// The listing filter: no reader keeps every row, a row on no thread is the
/// workspace's, and any other row needs [`readable_thread`].
pub(crate) fn readable_row(
    thread_column: &str,
    workspace: &str,
    member: &str,
    dm_channel: &str,
) -> String {
    let readable = readable_thread(thread_column, workspace, member, dm_channel);
    format!("({member} IS NULL OR {thread_column} IS NULL OR {readable})")
}
