//! Who may read a thread, as a SQL condition, for listings that must filter
//! in the query rather than after it.
//!
//! This is the query form of `maidan_auth::authorize_thread`, and the two
//! must agree: the thread's channel is in the caller's workspace, the caller
//! is a member of that workspace, and then a
//! `__dm__` thread needs the caller in its DM or group DM, a private channel
//! needs a `channel_members` row, and any other channel is open. Filtering a
//! page after the query costs a batch per hidden row and makes page size
//! depend on what the caller cannot see. `claim_next` and the queue counts
//! read through it too (`claim_next::readable_by`). `tests/thread_access.rs`
//! checks the two against each other on both backends.

/// A condition true when `member` may read the thread `thread` names.
/// `thread` is a SQL expression; `workspace`, `member` and `dm_channel` are
/// placeholders (with any cast the backend needs) bound to the caller's
/// workspace, its member id and [`maidan_types::DM_CHANNEL_NAME`]. The same
/// SQL runs on Postgres and SQLite.
///
/// The reader must be a member of the thread's own workspace, so the
/// condition holds on its own rather than trusting each caller to have pinned
/// `workspace` to the reader's: a public channel is open only to its
/// workspace. `member` appears once, in the one-row `rt_reader` table, so a
/// caller with positional `?` placeholders binds the member once.
pub fn readable_thread(thread: &str, workspace: &str, member: &str, dm_channel: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM (SELECT {member} AS reader_id) rt_reader
                 JOIN maidan_threads rt_thread ON rt_thread.id = {thread}
                 JOIN maidan_channels rt_channel ON rt_channel.id = rt_thread.channel_id
                 JOIN maidan_members rt_reader_row
                      ON rt_reader_row.id = rt_reader.reader_id
                     AND rt_reader_row.workspace_id = rt_channel.workspace_id
                 WHERE rt_channel.workspace_id = {workspace}
                   AND CASE
                         WHEN rt_channel.name = {dm_channel} THEN
                              EXISTS (SELECT 1 FROM maidan_dm_conversations rt_dm
                                      WHERE rt_dm.thread_id = rt_thread.id
                                        AND rt_reader.reader_id IN (rt_dm.member_low_id, rt_dm.member_high_id))
                           OR EXISTS (SELECT 1 FROM maidan_group_dm_conversations rt_group
                                      JOIN maidan_group_dm_members rt_group_member
                                           ON rt_group_member.group_dm_id = rt_group.id
                                      WHERE rt_group.thread_id = rt_thread.id
                                        AND rt_group_member.member_id = rt_reader.reader_id)
                         WHEN rt_channel.private THEN
                              EXISTS (SELECT 1 FROM maidan_channel_members rt_member
                                      WHERE rt_member.channel_id = rt_channel.id
                                        AND rt_member.member_id = rt_reader.reader_id)
                         ELSE TRUE
                       END)"
    )
}

/// The listing filter: no reader keeps every row, a row on no thread is the
/// workspace's, and any other row needs [`readable_thread`].
pub fn readable_row(
    thread_column: &str,
    workspace: &str,
    member: &str,
    dm_channel: &str,
) -> String {
    let readable = readable_thread(thread_column, workspace, member, dm_channel);
    format!("({member} IS NULL OR {thread_column} IS NULL OR {readable})")
}
