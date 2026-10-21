use sqlx::{PgPool, SqlitePool};

use crate::error::StoreError;
use std::collections::HashSet;

const POSTGRES_UP_V1: &str = include_str!("../../../migrations/postgres/0001_core_up.sql");
const POSTGRES_UP_V2: &str = include_str!("../../../migrations/postgres/0002_search.sql");
const POSTGRES_UP_V3: &str = include_str!("../../../migrations/postgres/0003_embeddings.sql");
const POSTGRES_UP_V4: &str = include_str!("../../../migrations/postgres/0004_thread_fsm.sql");
const POSTGRES_UP_V5: &str = include_str!("../../../migrations/postgres/0005_parent_threads.sql");
const POSTGRES_UP_V6: &str = include_str!("../../../migrations/postgres/0006_event_log.sql");
const POSTGRES_UP_V7: &str = include_str!("../../../migrations/postgres/0007_artifact_kinds.sql");
const POSTGRES_UP_V8: &str = include_str!("../../../migrations/postgres/0008_api_tokens.sql");
const POSTGRES_UP_V9: &str = include_str!("../../../migrations/postgres/0009_federation_peers.sql");
const POSTGRES_UP_V10: &str =
    include_str!("../../../migrations/postgres/0010_peer_outbound_secret.sql");
const POSTGRES_UP_V11: &str =
    include_str!("../../../migrations/postgres/0011_peer_remote_workspace.sql");
const POSTGRES_UP_V12: &str = include_str!("../../../migrations/postgres/0012_oidc_sessions.sql");
const POSTGRES_UP_V13: &str = include_str!("../../../migrations/postgres/0013_outbox.sql");
const POSTGRES_UP_V14: &str =
    include_str!("../../../migrations/postgres/0014_outbox_quarantine.sql");
const POSTGRES_UP_V15: &str = include_str!("../../../migrations/postgres/0015_delivery_cursor.sql");
const POSTGRES_UP_V16: &str =
    include_str!("../../../migrations/postgres/0016_dm_conversations.sql");
const POSTGRES_UP_V17: &str = include_str!("../../../migrations/postgres/0017_inbox_cursor.sql");
const POSTGRES_UP_V18: &str = include_str!("../../../migrations/postgres/0018_reactions_pins.sql");
const POSTGRES_UP_V19: &str = include_str!("../../../migrations/postgres/0019_message_edits.sql");
const POSTGRES_UP_V20: &str =
    include_str!("../../../migrations/postgres/0020_embedding_models.sql");
const POSTGRES_UP_V21: &str = include_str!("../../../migrations/postgres/0021_webhooks.sql");
const POSTGRES_UP_V22: &str = include_str!("../../../migrations/postgres/0022_slash_commands.sql");
const POSTGRES_UP_V23: &str = include_str!("../../../migrations/postgres/0023_fsm_hooks.sql");
const POSTGRES_UP_V24: &str = include_str!("../../../migrations/postgres/0024_token_quotas.sql");
const POSTGRES_UP_V25: &str = include_str!("../../../migrations/postgres/0025_agent_apps.sql");
const POSTGRES_UP_V26: &str =
    include_str!("../../../migrations/postgres/0026_automation_deliveries.sql");
const POSTGRES_UP_V27: &str =
    include_str!("../../../migrations/postgres/0027_a2a_push_and_tasks.sql");
const POSTGRES_UP_V28: &str =
    include_str!("../../../migrations/postgres/0028_group_dm_and_mention_webhook.sql");
const POSTGRES_UP_V29: &str = include_str!("../../../migrations/postgres/0029_oauth_codes.sql");
const POSTGRES_UP_V30: &str = include_str!("../../../migrations/postgres/0030_reindex_jobs.sql");
const POSTGRES_UP_V31: &str =
    include_str!("../../../migrations/postgres/0031_events_inserted_at.sql");
const POSTGRES_UP_V32: &str = include_str!("../../../migrations/postgres/0032_channel_members.sql");
const POSTGRES_UP_V33: &str =
    include_str!("../../../migrations/postgres/0033_threads_assignee.sql");
const POSTGRES_UP_V34: &str = include_str!("../../../migrations/postgres/0034_message_content.sql");
const POSTGRES_UP_V35: &str =
    include_str!("../../../migrations/postgres/0035_thread_claim_lease.sql");
const POSTGRES_UP_V36: &str =
    include_str!("../../../migrations/postgres/0036_artifact_workspace_refs.sql");
const POSTGRES_UP_V37: &str =
    include_str!("../../../migrations/postgres/0037_thread_dependencies.sql");
const POSTGRES_UP_V38: &str = include_str!("../../../migrations/postgres/0038_task_schedules.sql");
const POSTGRES_UP_V39: &str = include_str!("../../../migrations/postgres/0039_member_skills.sql");
const POSTGRES_UP_V40: &str =
    include_str!("../../../migrations/postgres/0040_thread_required_skills.sql");
const POSTGRES_UP_V41: &str = include_str!("../../../migrations/postgres/0041_thread_results.sql");
const POSTGRES_UP_V42: &str = include_str!("../../../migrations/postgres/0042_notifications.sql");
const POSTGRES_UP_V43: &str =
    include_str!("../../../migrations/postgres/0043_notification_dedup.sql");
const POSTGRES_UP_V44: &str =
    include_str!("../../../migrations/postgres/0044_notification_prefs.sql");
const POSTGRES_UP_V45: &str = include_str!("../../../migrations/postgres/0045_follows.sql");
const POSTGRES_UP_V46: &str = include_str!("../../../migrations/postgres/0046_member_emails.sql");
const POSTGRES_UP_V47: &str =
    include_str!("../../../migrations/postgres/0047_member_last_seen.sql");
const POSTGRES_UP_V48: &str = include_str!("../../../migrations/postgres/0048_email_digest.sql");
const POSTGRES_UP_V49: &str =
    include_str!("../../../migrations/postgres/0049_a2a_task_push_configs.sql");
const POSTGRES_UP_V50: &str = include_str!("../../../migrations/postgres/0050_mail_outbox.sql");
const POSTGRES_UP_V51: &str =
    include_str!("../../../migrations/postgres/0051_slack_channel_links.sql");
const POSTGRES_UP_V52: &str =
    include_str!("../../../migrations/postgres/0052_github_issue_links.sql");
const POSTGRES_UP_V53: &str = include_str!("../../../migrations/postgres/0053_glossary_terms.sql");
const POSTGRES_UP_V54: &str = include_str!("../../../migrations/postgres/0054_vote_confidence.sql");
const POSTGRES_UP_V55: &str =
    include_str!("../../../migrations/postgres/0055_artifact_kind_context_snapshot.sql");
const POSTGRES_UP_V56: &str = include_str!("../../../migrations/postgres/0056_approval_gates.sql");
const POSTGRES_UP_V57: &str =
    include_str!("../../../migrations/postgres/0057_thread_claim_lease_id.sql");
const POSTGRES_UP_V58: &str =
    include_str!("../../../migrations/postgres/0058_thread_work_started_at.sql");
const POSTGRES_UP_V59: &str = include_str!("../../../migrations/postgres/0059_threads_owner.sql");
const POSTGRES_UP_V60: &str = include_str!("../../../migrations/postgres/0060_thread_steer.sql");
const POSTGRES_UP_V61: &str = include_str!("../../../migrations/postgres/0061_thread_mutes.sql");
const POSTGRES_UP_V62: &str = include_str!("../../../migrations/postgres/0062_channel_mutes.sql");
const POSTGRES_UP_V63: &str = include_str!("../../../migrations/postgres/0063_thread_budgets.sql");
const POSTGRES_UP_V64: &str = include_str!("../../../migrations/postgres/0064_agent_work_dlq.sql");
const POSTGRES_UP_V65: &str =
    include_str!("../../../migrations/postgres/0065_notification_snooze.sql");
const POSTGRES_UP_V66: &str = include_str!("../../../migrations/postgres/0066_wip_limits.sql");
const POSTGRES_UP_V67: &str =
    include_str!("../../../migrations/postgres/0067_thread_unclaimable.sql");
const POSTGRES_UP_V68: &str = include_str!("../../../migrations/postgres/0068_thread_waits.sql");
const POSTGRES_UP_V69: &str =
    include_str!("../../../migrations/postgres/0069_thread_priorities.sql");
const POSTGRES_UP_V70: &str = include_str!("../../../migrations/postgres/0070_legal_holds.sql");
const POSTGRES_UP_V71: &str =
    include_str!("../../../migrations/postgres/0071_push_subscriptions.sql");
const POSTGRES_UP_V72: &str = include_str!("../../../migrations/postgres/0072_scim_users.sql");
const POSTGRES_UP_V73: &str = include_str!("../../../migrations/postgres/0073_recipes.sql");
const POSTGRES_UP_V74: &str = include_str!("../../../migrations/postgres/0074_recipe_runs.sql");
const POSTGRES_UP_V75: &str =
    include_str!("../../../migrations/postgres/0075_task_schedule_recipe.sql");
const POSTGRES_UP_V76: &str = include_str!("../../../migrations/postgres/0076_secrets.sql");
const POSTGRES_UP_V77: &str = include_str!("../../../migrations/postgres/0077_member_freezes.sql");
const POSTGRES_UP_V78: &str = include_str!("../../../migrations/postgres/0078_memory_blocks.sql");
const POSTGRES_UP_V79: &str = include_str!("../../../migrations/postgres/0079_thread_reviews.sql");
const POSTGRES_UP_V80: &str = include_str!("../../../migrations/postgres/0080_spawn_budgets.sql");
const POSTGRES_UP_V81: &str = include_str!("../../../migrations/postgres/0081_github_link_cap.sql");
const POSTGRES_UP_V82: &str = include_str!("../../../migrations/postgres/0082_egress_outbox.sql");
const POSTGRES_UP_V83: &str =
    include_str!("../../../migrations/postgres/0083_projector_link_disabled.sql");
const POSTGRES_UP_V84: &str = include_str!("../../../migrations/postgres/0084_egress_targets.sql");
const POSTGRES_UP_V85: &str =
    include_str!("../../../migrations/postgres/0085_result_deliveries.sql");
const POSTGRES_UP_V86: &str =
    include_str!("../../../migrations/postgres/0086_egress_outbox_kind.sql");
const POSTGRES_UP_V87: &str =
    include_str!("../../../migrations/postgres/0087_thread_result_kind.sql");
const POSTGRES_UP_V88: &str =
    include_str!("../../../migrations/postgres/0088_thread_land_gate.sql");
const POSTGRES_UP_V89: &str = include_str!("../../../migrations/postgres/0089_thread_blocks.sql");
const POSTGRES_UP_V90: &str = include_str!("../../../migrations/postgres/0090_thread_lineage.sql");
const POSTGRES_UP_V91: &str = include_str!("../../../migrations/postgres/0091_event_chain.sql");
const POSTGRES_UP_V92: &str =
    include_str!("../../../migrations/postgres/0092_federated_origin_link.sql");
const POSTGRES_UP_V93: &str =
    include_str!("../../../migrations/postgres/0093_workspace_handles.sql");
const POSTGRES_UP_V94: &str =
    include_str!("../../../migrations/postgres/0094_federated_verified_link.sql");
const POSTGRES_UP_V95: &str = include_str!("../../../migrations/postgres/0095_outbox_claim.sql");
const POSTGRES_UP_V96: &str = include_str!("../../../migrations/postgres/0096_mail_workspace.sql");
const POSTGRES_UP_V97: &str = include_str!("../../../migrations/postgres/0097_thread_workers.sql");
const POSTGRES_UP_V98: &str = include_str!("../../../migrations/postgres/0098_token_parent.sql");
const POSTGRES_UP_V99: &str = include_str!("../../../migrations/postgres/0099_tap_cursor.sql");
const POSTGRES_UP_V100: &str = include_str!("../../../migrations/postgres/0100_member_follows.sql");
const POSTGRES_UP_V101: &str = include_str!("../../../migrations/postgres/0101_share_tickets.sql");
const POSTGRES_UP_V102: &str = include_str!("../../../migrations/postgres/0102_usage_ledger.sql");
const POSTGRES_UP_V103: &str =
    include_str!("../../../migrations/postgres/0103_delegation_grants.sql");
const POSTGRES_UP_V104: &str =
    include_str!("../../../migrations/postgres/0104_delegated_tokens.sql");
const POSTGRES_UP_V105: &str =
    include_str!("../../../migrations/postgres/0105_audit_attribution.sql");
const POSTGRES_UP_V106: &str =
    include_str!("../../../migrations/postgres/0106_attestation_actors.sql");
const POSTGRES_UP_V107: &str =
    include_str!("../../../migrations/postgres/0107_delegation_policies.sql");
const POSTGRES_UP_V108: &str =
    include_str!("../../../migrations/postgres/0108_audit_keeps_its_actors.sql");
const POSTGRES_UP_V109: &str =
    include_str!("../../../migrations/postgres/0109_artifact_ref_metadata.sql");
const POSTGRES_UP_V110: &str =
    include_str!("../../../migrations/postgres/0110_review_dismissal.sql");
const POSTGRES_UP_V111: &str =
    include_str!("../../../migrations/postgres/0111_preserved_messages.sql");
const POSTGRES_UP_V112: &str =
    include_str!("../../../migrations/postgres/0112_legal_holds_per_matter.sql");
const POSTGRES_UP_V113: &str = include_str!("../../../migrations/postgres/0113_content_keys.sql");
const POSTGRES_UP_V114: &str =
    include_str!("../../../migrations/postgres/0114_mail_content_key.sql");
const POSTGRES_UP_V115: &str =
    include_str!("../../../migrations/postgres/0115_a2a_v1_conformance.sql");
const POSTGRES_UP_V116: &str =
    include_str!("../../../migrations/postgres/0116_approval_gate_millisecond_positions.sql");
const POSTGRES_UP_V117: &str =
    include_str!("../../../migrations/postgres/0117_artifact_reap_leases.sql");
const POSTGRES_UP_V118: &str =
    include_str!("../../../migrations/postgres/0118_idempotency_keys.sql");
const POSTGRES_UP_V119: &str =
    include_str!("../../../migrations/postgres/0119_claim_lease_deadline_index.sql");
const POSTGRES_UP_V120: &str =
    include_str!("../../../migrations/postgres/0120_claim_acknowledgement_window.sql");
const POSTGRES_UP_V121: &str =
    include_str!("../../../migrations/postgres/0121_decision_history.sql");

const POSTGRES_UP_V122: &str = include_str!("../../../migrations/postgres/0122_trace_context.sql");
const POSTGRES_UP_V123: &str =
    include_str!("../../../migrations/postgres/0123_audit_workspace.sql");
const POSTGRES_UP_V124: &str =
    include_str!("../../../migrations/postgres/0124_thread_last_worker.sql");
const POSTGRES_UP_V126: &str =
    include_str!("../../../migrations/postgres/0126_mcp_resource_subscriptions.sql");
const POSTGRES_UP_V127: &str = include_str!("../../../migrations/postgres/0127_scim_groups.sql");
const POSTGRES_UP_V128: &str =
    include_str!("../../../migrations/postgres/0128_artifact_filename.sql");
const POSTGRES_UP_V129: &str =
    include_str!("../../../migrations/postgres/0129_session_from_token.sql");
const POSTGRES_UP_V130: &str =
    include_str!("../../../migrations/postgres/0130_secret_egress_hosts.sql");
const POSTGRES_UP_V131: &str =
    include_str!("../../../migrations/postgres/0131_a2a_task_threads.sql");
const POSTGRES_UP_V132: &str =
    include_str!("../../../migrations/postgres/0132_thread_budget_wall_charge.sql");
const POSTGRES_UP_V134: &str =
    include_str!("../../../migrations/postgres/0134_retention_policies.sql");
const POSTGRES_UP_V135: &str =
    include_str!("../../../migrations/postgres/0135_member_handle_ci.sql");
const POSTGRES_UP_V136: &str = include_str!("../../../migrations/postgres/0136_artifact_erase.sql");
const POSTGRES_UP_V137: &str =
    include_str!("../../../migrations/postgres/0137_slack_link_thread_index.sql");
const POSTGRES_UP_V138: &str =
    include_str!("../../../migrations/postgres/0138_web_push_outbox.sql");
const POSTGRES_UP_V139: &str =
    include_str!("../../../migrations/postgres/0139_usage_cache_pricing.sql");
const POSTGRES_UP_V140: &str =
    include_str!("../../../migrations/postgres/0140_thread_block_note.sql");
const POSTGRES_UP_V141: &str = include_str!("../../../migrations/postgres/0141_thread_status.sql");
const POSTGRES_UP_V142: &str =
    include_str!("../../../migrations/postgres/0142_thread_description.sql");
const POSTGRES_UP_V143: &str =
    include_str!("../../../migrations/postgres/0143_one_verdict_per_member.sql");
const POSTGRES_UP_V144: &str =
    include_str!("../../../migrations/postgres/0144_thread_state_version.sql");
const POSTGRES_UP_V145: &str = include_str!("../../../migrations/postgres/0145_review_packets.sql");
const POSTGRES_UP_V146: &str = include_str!("../../../migrations/postgres/0146_bound_verdicts.sql");
const POSTGRES_UP_V147: &str =
    include_str!("../../../migrations/postgres/0147_evidence_actors.sql");
const POSTGRES_UP_V148: &str =
    include_str!("../../../migrations/postgres/0148_approval_decide.sql");
const POSTGRES_UP_V149: &str =
    include_str!("../../../migrations/postgres/0149_approval_confirm_link_ttl.sql");
const POSTGRES_UP_V150: &str =
    include_str!("../../../migrations/postgres/0150_oauth_clients_codes_grants.sql");
const POSTGRES_UP_V151: &str =
    include_str!("../../../migrations/postgres/0151_oauth_pending_requests.sql");
const POSTGRES_UP_V152: &str =
    include_str!("../../../migrations/postgres/0152_oauth_grant_live_unique.sql");
const POSTGRES_UP_V153: &str =
    include_str!("../../../migrations/postgres/0153_oauth_refresh_tokens.sql");
const POSTGRES_UP_V160: &str =
    include_str!("../../../migrations/postgres/0160_decided_via_source.sql");
const POSTGRES_UP_V161: &str =
    include_str!("../../../migrations/postgres/0161_session_identity.sql");
// 0162 is Postgres only: SQLite keeps the event log in one table.
const POSTGRES_UP_V162: &str =
    include_str!("../../../migrations/postgres/0162_partition_event_log.sql");
const POSTGRES_UP_V163: &str =
    include_str!("../../../migrations/postgres/0163_oidc_front_door.sql");
const POSTGRES_UP_V164: &str =
    include_str!("../../../migrations/postgres/0164_event_channel_kind_index.sql");
const POSTGRES_UP_V165: &str =
    include_str!("../../../migrations/postgres/0165_partition_audit_log.sql");
const POSTGRES_UP_V166: &str =
    include_str!("../../../migrations/postgres/0166_partition_delivery_tables.sql");
const POSTGRES_UP_V167: &str = include_str!("../../../migrations/postgres/0167_workspace_keys.sql");
const POSTGRES_UP_V168: &str =
    include_str!("../../../migrations/postgres/0168_claim_footprints.sql");
const POSTGRES_UP_V170: &str =
    include_str!("../../../migrations/postgres/0170_media_retention_quota.sql");
const POSTGRES_UP_V171: &str = include_str!("../../../migrations/postgres/0171_repo_pauses.sql");
const POSTGRES_UP_V172: &str =
    include_str!("../../../migrations/postgres/0172_oauth_client_metadata_url.sql");
const POSTGRES_UP_V173: &str = include_str!("../../../migrations/postgres/0173_claim_slots.sql");
const POSTGRES_UP_V174: &str = include_str!("../../../migrations/postgres/0174_change_metrics.sql");
const POSTGRES_UP_V175: &str =
    include_str!("../../../migrations/postgres/0175_thread_attributes.sql");
const POSTGRES_UP_V176: &str = include_str!("../../../migrations/postgres/0176_change_notices.sql");
const POSTGRES_UP_V177: &str =
    include_str!("../../../migrations/postgres/0177_change_pull_checked.sql");
const POSTGRES_UP_V178: &str =
    include_str!("../../../migrations/postgres/0178_waitlist_and_signup.sql");
const POSTGRES_UP_V180: &str = include_str!("../../../migrations/postgres/0180_thread_cancel.sql");
const POSTGRES_UP_V181: &str =
    include_str!("../../../migrations/postgres/0181_artifact_reap_retries.sql");
const POSTGRES_UP_V182: &str =
    include_str!("../../../migrations/postgres/0182_change_round_rebase.sql");
const POSTGRES_UP_V183: &str =
    include_str!("../../../migrations/postgres/0183_change_round_previous_head.sql");
const POSTGRES_UP_V184: &str = include_str!("../../../migrations/postgres/0184_thread_plans.sql");
const POSTGRES_UP_V185: &str =
    include_str!("../../../migrations/postgres/0185_result_delivery_template_warnings.sql");
const POSTGRES_UP_V186: &str =
    include_str!("../../../migrations/postgres/0186_change_pull_linear_issue.sql");
const POSTGRES_UP_V187: &str =
    include_str!("../../../migrations/postgres/0187_change_pull_outcomes.sql");
const POSTGRES_UP_V188: &str =
    include_str!("../../../migrations/postgres/0188_linear_claim_comments.sql");
const POSTGRES_UP_V189: &str =
    include_str!("../../../migrations/postgres/0189_thread_handoffs.sql");
const POSTGRES_UP_V190: &str =
    include_str!("../../../migrations/postgres/0190_workspace_default_engine.sql");
const POSTGRES_UP_V191: &str = include_str!("../../../migrations/postgres/0191_pull_ci.sql");
const POSTGRES_UP_V192: &str = include_str!("../../../migrations/postgres/0192_pull_gate.sql");
const POSTGRES_UP_V193: &str =
    include_str!("../../../migrations/postgres/0193_pull_merge_queue.sql");
const POSTGRES_UP_V195: &str = include_str!("../../../migrations/postgres/0195_deploys.sql");
const SQLITE_UP_V1: &str = include_str!("../../../migrations/sqlite/0001_core_up.sql");
const SQLITE_UP_V2: &str = include_str!("../../../migrations/sqlite/0002_search.sql");
const SQLITE_UP_V3: &str = include_str!("../../../migrations/sqlite/0003_embeddings.sql");
const SQLITE_UP_V4: &str = include_str!("../../../migrations/sqlite/0004_thread_fsm.sql");
const SQLITE_UP_V5: &str = include_str!("../../../migrations/sqlite/0005_parent_threads.sql");
const SQLITE_UP_V6: &str = include_str!("../../../migrations/sqlite/0006_event_log.sql");
const SQLITE_UP_V7: &str = include_str!("../../../migrations/sqlite/0007_artifact_kinds.sql");
const SQLITE_UP_V8: &str = include_str!("../../../migrations/sqlite/0008_api_tokens.sql");
const SQLITE_UP_V9: &str = include_str!("../../../migrations/sqlite/0009_federation_peers.sql");
const SQLITE_UP_V10: &str =
    include_str!("../../../migrations/sqlite/0010_peer_outbound_secret.sql");
const SQLITE_UP_V11: &str =
    include_str!("../../../migrations/sqlite/0011_peer_remote_workspace.sql");
const SQLITE_UP_V12: &str = include_str!("../../../migrations/sqlite/0012_oidc_sessions.sql");
const SQLITE_UP_V13: &str = include_str!("../../../migrations/sqlite/0013_outbox.sql");
const SQLITE_UP_V14: &str = include_str!("../../../migrations/sqlite/0014_dm_conversations.sql");
const SQLITE_UP_V15: &str = include_str!("../../../migrations/sqlite/0015_inbox_cursor.sql");
const SQLITE_UP_V16: &str = include_str!("../../../migrations/sqlite/0016_reactions_pins.sql");
const SQLITE_UP_V17: &str = include_str!("../../../migrations/sqlite/0017_message_edits.sql");
const SQLITE_UP_V18: &str = include_str!("../../../migrations/sqlite/0018_embedding_models.sql");
const SQLITE_UP_V19: &str = include_str!("../../../migrations/sqlite/0019_webhooks.sql");
const SQLITE_UP_V20: &str = include_str!("../../../migrations/sqlite/0020_slash_commands.sql");
const SQLITE_UP_V21: &str = include_str!("../../../migrations/sqlite/0021_fsm_hooks.sql");
const SQLITE_UP_V22: &str = include_str!("../../../migrations/sqlite/0022_token_quotas.sql");
const SQLITE_UP_V23: &str = include_str!("../../../migrations/sqlite/0023_delivery_cursor.sql");
const SQLITE_UP_V24: &str = include_str!("../../../migrations/sqlite/0024_agent_apps.sql");
const SQLITE_UP_V25: &str =
    include_str!("../../../migrations/sqlite/0025_automation_deliveries.sql");
const SQLITE_UP_V26: &str = include_str!("../../../migrations/sqlite/0026_a2a_push_and_tasks.sql");
const SQLITE_UP_V27: &str =
    include_str!("../../../migrations/sqlite/0027_group_dm_and_mention_webhook.sql");
const SQLITE_UP_V28: &str = include_str!("../../../migrations/sqlite/0028_oauth_codes.sql");
const SQLITE_UP_V29: &str = include_str!("../../../migrations/sqlite/0029_reindex_jobs.sql");
const SQLITE_UP_V30: &str = include_str!("../../../migrations/sqlite/0030_events_inserted_at.sql");
const SQLITE_UP_V31: &str = include_str!("../../../migrations/sqlite/0031_channel_members.sql");
const SQLITE_UP_V32: &str = include_str!("../../../migrations/sqlite/0032_threads_assignee.sql");
const SQLITE_UP_V33: &str = include_str!("../../../migrations/sqlite/0033_message_content.sql");
const SQLITE_UP_V34: &str = include_str!("../../../migrations/sqlite/0034_thread_claim_lease.sql");
const SQLITE_UP_V35: &str =
    include_str!("../../../migrations/sqlite/0035_artifact_workspace_refs.sql");
const SQLITE_UP_V36: &str = include_str!("../../../migrations/sqlite/0036_thread_dependencies.sql");
const SQLITE_UP_V37: &str = include_str!("../../../migrations/sqlite/0037_task_schedules.sql");
const SQLITE_UP_V38: &str = include_str!("../../../migrations/sqlite/0038_member_skills.sql");
const SQLITE_UP_V39: &str =
    include_str!("../../../migrations/sqlite/0039_thread_required_skills.sql");
const SQLITE_UP_V40: &str = include_str!("../../../migrations/sqlite/0040_thread_results.sql");
const SQLITE_UP_V41: &str = include_str!("../../../migrations/sqlite/0041_notifications.sql");
const SQLITE_UP_V42: &str = include_str!("../../../migrations/sqlite/0042_notification_dedup.sql");
const SQLITE_UP_V43: &str = include_str!("../../../migrations/sqlite/0043_notification_prefs.sql");
const SQLITE_UP_V44: &str = include_str!("../../../migrations/sqlite/0044_follows.sql");
const SQLITE_UP_V45: &str = include_str!("../../../migrations/sqlite/0045_member_emails.sql");
const SQLITE_UP_V46: &str = include_str!("../../../migrations/sqlite/0046_member_last_seen.sql");
const SQLITE_UP_V47: &str = include_str!("../../../migrations/sqlite/0047_email_digest.sql");
const SQLITE_UP_V48: &str =
    include_str!("../../../migrations/sqlite/0048_a2a_task_push_configs.sql");
const SQLITE_UP_V49: &str = include_str!("../../../migrations/sqlite/0049_mail_outbox.sql");
const SQLITE_UP_V50: &str = include_str!("../../../migrations/sqlite/0050_slack_channel_links.sql");
const SQLITE_UP_V51: &str = include_str!("../../../migrations/sqlite/0051_github_issue_links.sql");
const SQLITE_UP_V52: &str = include_str!("../../../migrations/sqlite/0052_glossary_terms.sql");
const SQLITE_UP_V53: &str = include_str!("../../../migrations/sqlite/0053_vote_confidence.sql");
const SQLITE_UP_V54: &str =
    include_str!("../../../migrations/sqlite/0054_artifact_kind_context_snapshot.sql");
const SQLITE_UP_V55: &str = include_str!("../../../migrations/sqlite/0055_approval_gates.sql");
const SQLITE_UP_V56: &str =
    include_str!("../../../migrations/sqlite/0056_thread_claim_lease_id.sql");
const SQLITE_UP_V57: &str =
    include_str!("../../../migrations/sqlite/0057_thread_work_started_at.sql");
const SQLITE_UP_V58: &str = include_str!("../../../migrations/sqlite/0058_threads_owner.sql");
const SQLITE_UP_V59: &str = include_str!("../../../migrations/sqlite/0059_thread_steer.sql");
const SQLITE_UP_V60: &str = include_str!("../../../migrations/sqlite/0060_thread_mutes.sql");
const SQLITE_UP_V61: &str = include_str!("../../../migrations/sqlite/0061_channel_mutes.sql");
const SQLITE_UP_V62: &str = include_str!("../../../migrations/sqlite/0062_thread_budgets.sql");
const SQLITE_UP_V63: &str = include_str!("../../../migrations/sqlite/0063_agent_work_dlq.sql");
const SQLITE_UP_V64: &str = include_str!("../../../migrations/sqlite/0064_notification_snooze.sql");
const SQLITE_UP_V65: &str = include_str!("../../../migrations/sqlite/0065_wip_limits.sql");
const SQLITE_UP_V66: &str = include_str!("../../../migrations/sqlite/0066_thread_unclaimable.sql");
const SQLITE_UP_V67: &str = include_str!("../../../migrations/sqlite/0067_thread_waits.sql");
const SQLITE_UP_V68: &str = include_str!("../../../migrations/sqlite/0068_thread_priorities.sql");
const SQLITE_UP_V69: &str = include_str!("../../../migrations/sqlite/0069_legal_holds.sql");
const SQLITE_UP_V70: &str = include_str!("../../../migrations/sqlite/0070_push_subscriptions.sql");
const SQLITE_UP_V71: &str = include_str!("../../../migrations/sqlite/0071_scim_users.sql");
const SQLITE_UP_V72: &str = include_str!("../../../migrations/sqlite/0072_recipes.sql");
const SQLITE_UP_V73: &str = include_str!("../../../migrations/sqlite/0073_recipe_runs.sql");
const SQLITE_UP_V74: &str =
    include_str!("../../../migrations/sqlite/0074_task_schedule_recipe.sql");
const SQLITE_UP_V75: &str = include_str!("../../../migrations/sqlite/0075_secrets.sql");
const SQLITE_UP_V76: &str = include_str!("../../../migrations/sqlite/0076_member_freezes.sql");
const SQLITE_UP_V77: &str = include_str!("../../../migrations/sqlite/0077_memory_blocks.sql");
const SQLITE_UP_V78: &str = include_str!("../../../migrations/sqlite/0078_thread_reviews.sql");
const SQLITE_UP_V79: &str = include_str!("../../../migrations/sqlite/0079_spawn_budgets.sql");
const SQLITE_UP_V80: &str = include_str!("../../../migrations/sqlite/0080_github_link_cap.sql");
const SQLITE_UP_V81: &str = include_str!("../../../migrations/sqlite/0081_egress_outbox.sql");
const SQLITE_UP_V82: &str =
    include_str!("../../../migrations/sqlite/0082_projector_link_disabled.sql");
const SQLITE_UP_V83: &str = include_str!("../../../migrations/sqlite/0083_egress_targets.sql");
const SQLITE_UP_V84: &str = include_str!("../../../migrations/sqlite/0084_result_deliveries.sql");
const SQLITE_UP_V85: &str = include_str!("../../../migrations/sqlite/0085_egress_outbox_kind.sql");
const SQLITE_UP_V86: &str = include_str!("../../../migrations/sqlite/0086_thread_result_kind.sql");
const SQLITE_UP_V87: &str = include_str!("../../../migrations/sqlite/0087_thread_land_gate.sql");
const SQLITE_UP_V88: &str = include_str!("../../../migrations/sqlite/0088_thread_blocks.sql");
const SQLITE_UP_V89: &str = include_str!("../../../migrations/sqlite/0089_thread_lineage.sql");
const SQLITE_UP_V90: &str = include_str!("../../../migrations/sqlite/0090_event_chain.sql");
const SQLITE_UP_V91: &str =
    include_str!("../../../migrations/sqlite/0091_federated_origin_link.sql");
const SQLITE_UP_V92: &str = include_str!("../../../migrations/sqlite/0092_workspace_handles.sql");
const SQLITE_UP_V93: &str =
    include_str!("../../../migrations/sqlite/0093_federated_verified_link.sql");
const SQLITE_UP_V94: &str = include_str!("../../../migrations/sqlite/0094_outbox_claim.sql");
const SQLITE_UP_V95: &str = include_str!("../../../migrations/sqlite/0095_mail_workspace.sql");
const SQLITE_UP_V96: &str = include_str!("../../../migrations/sqlite/0096_thread_workers.sql");
const SQLITE_UP_V97: &str = include_str!("../../../migrations/sqlite/0097_token_parent.sql");
const SQLITE_UP_V98: &str = include_str!("../../../migrations/sqlite/0098_tap_cursor.sql");
const SQLITE_UP_V99: &str = include_str!("../../../migrations/sqlite/0099_member_follows.sql");
const SQLITE_UP_V100: &str = include_str!("../../../migrations/sqlite/0100_share_tickets.sql");
const SQLITE_UP_V101: &str = include_str!("../../../migrations/sqlite/0101_usage_ledger.sql");
const SQLITE_UP_V102: &str = include_str!("../../../migrations/sqlite/0102_delegation_grants.sql");
const SQLITE_UP_V103: &str = include_str!("../../../migrations/sqlite/0103_delegated_tokens.sql");
const SQLITE_UP_V104: &str = include_str!("../../../migrations/sqlite/0104_audit_attribution.sql");
const SQLITE_UP_V105: &str = include_str!("../../../migrations/sqlite/0105_attestation_actors.sql");
const SQLITE_UP_V106: &str =
    include_str!("../../../migrations/sqlite/0106_delegation_policies.sql");
const SQLITE_UP_V107: &str =
    include_str!("../../../migrations/sqlite/0107_audit_keeps_its_actors.sql");
const SQLITE_UP_V108: &str =
    include_str!("../../../migrations/sqlite/0108_artifact_ref_metadata.sql");
const SQLITE_UP_V109: &str = include_str!("../../../migrations/sqlite/0109_review_dismissal.sql");
const SQLITE_UP_V110: &str = include_str!("../../../migrations/sqlite/0110_preserved_messages.sql");
const SQLITE_UP_V111: &str =
    include_str!("../../../migrations/sqlite/0111_legal_holds_per_matter.sql");
const SQLITE_UP_V112: &str = include_str!("../../../migrations/sqlite/0112_content_keys.sql");
const SQLITE_UP_V113: &str = include_str!("../../../migrations/sqlite/0113_mail_content_key.sql");
const SQLITE_UP_V114: &str = include_str!("../../../migrations/sqlite/0114_a2a_v1_conformance.sql");
const SQLITE_UP_V115: &str =
    include_str!("../../../migrations/sqlite/0115_share_ticket_expiry_precision.sql");
const SQLITE_UP_V116: &str =
    include_str!("../../../migrations/sqlite/0116_approval_gate_millisecond_positions.sql");
const SQLITE_UP_V117: &str =
    include_str!("../../../migrations/sqlite/0117_artifact_reap_leases.sql");
const SQLITE_UP_V118: &str = include_str!("../../../migrations/sqlite/0118_idempotency_keys.sql");
const SQLITE_UP_V119: &str =
    include_str!("../../../migrations/sqlite/0119_claim_lease_deadline_index.sql");
const SQLITE_UP_V120: &str =
    include_str!("../../../migrations/sqlite/0120_claim_acknowledgement_window.sql");
const SQLITE_UP_V121: &str = include_str!("../../../migrations/sqlite/0121_decision_history.sql");

const SQLITE_UP_V122: &str = include_str!("../../../migrations/sqlite/0122_trace_context.sql");
const SQLITE_UP_V123: &str = include_str!("../../../migrations/sqlite/0123_audit_workspace.sql");
const SQLITE_UP_V124: &str = include_str!("../../../migrations/sqlite/0124_thread_last_worker.sql");
const SQLITE_UP_V126: &str =
    include_str!("../../../migrations/sqlite/0126_mcp_resource_subscriptions.sql");
const SQLITE_UP_V127: &str = include_str!("../../../migrations/sqlite/0127_scim_groups.sql");
const SQLITE_UP_V128: &str = include_str!("../../../migrations/sqlite/0128_artifact_filename.sql");
const SQLITE_UP_V129: &str = include_str!("../../../migrations/sqlite/0129_session_from_token.sql");
const SQLITE_UP_V130: &str =
    include_str!("../../../migrations/sqlite/0130_secret_egress_hosts.sql");
const SQLITE_UP_V131: &str = include_str!("../../../migrations/sqlite/0131_a2a_task_threads.sql");
const SQLITE_UP_V132: &str =
    include_str!("../../../migrations/sqlite/0132_thread_budget_wall_charge.sql");
const SQLITE_UP_V134: &str = include_str!("../../../migrations/sqlite/0134_retention_policies.sql");
const SQLITE_UP_V135: &str = include_str!("../../../migrations/sqlite/0135_member_handle_ci.sql");
const SQLITE_UP_V136: &str = include_str!("../../../migrations/sqlite/0136_artifact_erase.sql");
const SQLITE_UP_V137: &str =
    include_str!("../../../migrations/sqlite/0137_slack_link_thread_index.sql");
const SQLITE_UP_V138: &str = include_str!("../../../migrations/sqlite/0138_web_push_outbox.sql");
const SQLITE_UP_V139: &str =
    include_str!("../../../migrations/sqlite/0139_usage_cache_pricing.sql");
const SQLITE_UP_V140: &str = include_str!("../../../migrations/sqlite/0140_thread_block_note.sql");
const SQLITE_UP_V141: &str = include_str!("../../../migrations/sqlite/0141_thread_status.sql");
const SQLITE_UP_V142: &str = include_str!("../../../migrations/sqlite/0142_thread_description.sql");
const SQLITE_UP_V143: &str =
    include_str!("../../../migrations/sqlite/0143_one_verdict_per_member.sql");
const SQLITE_UP_V144: &str =
    include_str!("../../../migrations/sqlite/0144_thread_state_version.sql");
const SQLITE_UP_V145: &str = include_str!("../../../migrations/sqlite/0145_review_packets.sql");
const SQLITE_UP_V146: &str = include_str!("../../../migrations/sqlite/0146_bound_verdicts.sql");
const SQLITE_UP_V147: &str = include_str!("../../../migrations/sqlite/0147_evidence_actors.sql");
const SQLITE_UP_V148: &str = include_str!("../../../migrations/sqlite/0148_approval_decide.sql");
const SQLITE_UP_V149: &str =
    include_str!("../../../migrations/sqlite/0149_approval_confirm_link_ttl.sql");
const SQLITE_UP_V150: &str =
    include_str!("../../../migrations/sqlite/0150_oauth_clients_codes_grants.sql");
const SQLITE_UP_V151: &str =
    include_str!("../../../migrations/sqlite/0151_oauth_pending_requests.sql");
const SQLITE_UP_V152: &str =
    include_str!("../../../migrations/sqlite/0152_oauth_grant_live_unique.sql");
const SQLITE_UP_V153: &str =
    include_str!("../../../migrations/sqlite/0153_oauth_refresh_tokens.sql");
const SQLITE_UP_V160: &str = include_str!("../../../migrations/sqlite/0160_decided_via_source.sql");
const SQLITE_UP_V161: &str = include_str!("../../../migrations/sqlite/0161_session_identity.sql");
const SQLITE_UP_V163: &str = include_str!("../../../migrations/sqlite/0163_oidc_front_door.sql");
const SQLITE_UP_V164: &str =
    include_str!("../../../migrations/sqlite/0164_event_channel_kind_index.sql");
const SQLITE_UP_V167: &str = include_str!("../../../migrations/sqlite/0167_workspace_keys.sql");
const SQLITE_UP_V168: &str = include_str!("../../../migrations/sqlite/0168_claim_footprints.sql");
const SQLITE_UP_V170: &str =
    include_str!("../../../migrations/sqlite/0170_media_retention_quota.sql");
const SQLITE_UP_V171: &str = include_str!("../../../migrations/sqlite/0171_repo_pauses.sql");
const SQLITE_UP_V172: &str =
    include_str!("../../../migrations/sqlite/0172_oauth_client_metadata_url.sql");
const SQLITE_UP_V173: &str = include_str!("../../../migrations/sqlite/0173_claim_slots.sql");
const SQLITE_UP_V174: &str = include_str!("../../../migrations/sqlite/0174_change_metrics.sql");
const SQLITE_UP_V175: &str = include_str!("../../../migrations/sqlite/0175_thread_attributes.sql");
const SQLITE_UP_V176: &str = include_str!("../../../migrations/sqlite/0176_change_notices.sql");
const SQLITE_UP_V177: &str =
    include_str!("../../../migrations/sqlite/0177_change_pull_checked.sql");
const SQLITE_UP_V178: &str =
    include_str!("../../../migrations/sqlite/0178_waitlist_and_signup.sql");
const SQLITE_UP_V180: &str = include_str!("../../../migrations/sqlite/0180_thread_cancel.sql");
const SQLITE_UP_V181: &str =
    include_str!("../../../migrations/sqlite/0181_artifact_reap_retries.sql");
const SQLITE_UP_V182: &str =
    include_str!("../../../migrations/sqlite/0182_change_round_rebase.sql");
const SQLITE_UP_V183: &str =
    include_str!("../../../migrations/sqlite/0183_change_round_previous_head.sql");
const SQLITE_UP_V184: &str = include_str!("../../../migrations/sqlite/0184_thread_plans.sql");
const SQLITE_UP_V185: &str =
    include_str!("../../../migrations/sqlite/0185_result_delivery_template_warnings.sql");
const SQLITE_UP_V186: &str =
    include_str!("../../../migrations/sqlite/0186_change_pull_linear_issue.sql");
const SQLITE_UP_V187: &str =
    include_str!("../../../migrations/sqlite/0187_change_pull_outcomes.sql");
const SQLITE_UP_V188: &str =
    include_str!("../../../migrations/sqlite/0188_linear_claim_comments.sql");
const SQLITE_UP_V189: &str = include_str!("../../../migrations/sqlite/0189_thread_handoffs.sql");
const SQLITE_UP_V190: &str =
    include_str!("../../../migrations/sqlite/0190_workspace_default_engine.sql");
const SQLITE_UP_V191: &str = include_str!("../../../migrations/sqlite/0191_pull_ci.sql");
const SQLITE_UP_V192: &str = include_str!("../../../migrations/sqlite/0192_pull_gate.sql");
const SQLITE_UP_V193: &str = include_str!("../../../migrations/sqlite/0193_pull_merge_queue.sql");
const SQLITE_UP_V195: &str = include_str!("../../../migrations/sqlite/0195_deploys.sql");

/// Session advisory-lock key guarding boot-time migrations. Any constant works
/// as long as it is stable across replicas; this is the ASCII for `"migr"`,
/// chosen to be readable and unlikely to collide with application locks.
const MIGRATION_LOCK_KEY: i64 = 0x6D69_6772_i64;

/// Apply all Postgres migrations to the pool, in order, idempotently.
///
/// Tracks applied migrations in a `maidan_migrations` table. Calling
/// this repeatedly is safe; a migration only runs the first time it is
/// seen.
///
/// Boot-time migration is serialized across replicas by a Postgres session
/// advisory lock: when several replicas start against a fresh database they
/// would otherwise race on non-transactional DDL (e.g. concurrent
/// `CREATE EXTENSION`, which fails with a `pg_extension` unique violation). The
/// first replica holds the lock and migrates; the rest block, then observe the
/// migrations already applied and no-op.
pub async fn run_postgres_migrations(pool: &PgPool) -> Result<(), StoreError> {
    let mut lock_conn = pool.acquire().await?;
    // Exempt the migration session from the configured statement and lock
    // timeouts: the advisory-lock wait below can legitimately block while
    // another replica migrates, DDL must not be capped, and DDL may have to wait
    // on a lock a running replica holds during a rolling deploy.
    sqlx::query("SET statement_timeout = 0")
        .execute(&mut *lock_conn)
        .await?;
    sqlx::query("SET lock_timeout = 0")
        .execute(&mut *lock_conn)
        .await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await?;

    let result = apply_all_postgres(pool).await;

    // Release explicitly; dropping the connection would also release it, but an
    // explicit unlock returns the slot to the pool promptly.
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await;
    result
}

async fn apply_all_postgres(pool: &PgPool) -> Result<(), StoreError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS maidan_migrations (
            version BIGINT PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(pool)
    .await?;
    // One read of what is applied, rather than a query per migration: a boot
    // against a database across a network paid a round trip for each of them.
    // The advisory lock is held, so nothing applies a migration meanwhile.
    let applied = applied_versions_postgres(pool).await?;

    apply_postgres(pool, &applied, 1, POSTGRES_UP_V1).await?;
    apply_postgres(pool, &applied, 2, POSTGRES_UP_V2).await?;
    apply_postgres(pool, &applied, 3, POSTGRES_UP_V3).await?;
    apply_postgres(pool, &applied, 4, POSTGRES_UP_V4).await?;
    apply_postgres(pool, &applied, 5, POSTGRES_UP_V5).await?;
    apply_postgres(pool, &applied, 6, POSTGRES_UP_V6).await?;
    apply_postgres(pool, &applied, 7, POSTGRES_UP_V7).await?;
    apply_postgres(pool, &applied, 8, POSTGRES_UP_V8).await?;
    apply_postgres(pool, &applied, 9, POSTGRES_UP_V9).await?;
    apply_postgres(pool, &applied, 10, POSTGRES_UP_V10).await?;
    apply_postgres(pool, &applied, 11, POSTGRES_UP_V11).await?;
    apply_postgres(pool, &applied, 12, POSTGRES_UP_V12).await?;
    apply_postgres(pool, &applied, 13, POSTGRES_UP_V13).await?;
    apply_postgres(pool, &applied, 14, POSTGRES_UP_V14).await?;
    apply_postgres(pool, &applied, 15, POSTGRES_UP_V15).await?;
    apply_postgres(pool, &applied, 16, POSTGRES_UP_V16).await?;
    apply_postgres(pool, &applied, 17, POSTGRES_UP_V17).await?;
    apply_postgres(pool, &applied, 18, POSTGRES_UP_V18).await?;
    apply_postgres(pool, &applied, 19, POSTGRES_UP_V19).await?;
    apply_postgres(pool, &applied, 20, POSTGRES_UP_V20).await?;
    apply_postgres(pool, &applied, 21, POSTGRES_UP_V21).await?;
    apply_postgres(pool, &applied, 22, POSTGRES_UP_V22).await?;
    apply_postgres(pool, &applied, 23, POSTGRES_UP_V23).await?;
    apply_postgres(pool, &applied, 24, POSTGRES_UP_V24).await?;
    apply_postgres(pool, &applied, 25, POSTGRES_UP_V25).await?;
    apply_postgres(pool, &applied, 26, POSTGRES_UP_V26).await?;
    apply_postgres(pool, &applied, 27, POSTGRES_UP_V27).await?;
    apply_postgres(pool, &applied, 28, POSTGRES_UP_V28).await?;
    apply_postgres(pool, &applied, 29, POSTGRES_UP_V29).await?;
    apply_postgres(pool, &applied, 30, POSTGRES_UP_V30).await?;
    apply_postgres(pool, &applied, 31, POSTGRES_UP_V31).await?;
    apply_postgres(pool, &applied, 32, POSTGRES_UP_V32).await?;
    apply_postgres(pool, &applied, 33, POSTGRES_UP_V33).await?;
    apply_postgres(pool, &applied, 34, POSTGRES_UP_V34).await?;
    apply_postgres(pool, &applied, 35, POSTGRES_UP_V35).await?;
    apply_postgres(pool, &applied, 36, POSTGRES_UP_V36).await?;
    apply_postgres(pool, &applied, 37, POSTGRES_UP_V37).await?;
    apply_postgres(pool, &applied, 38, POSTGRES_UP_V38).await?;
    apply_postgres(pool, &applied, 39, POSTGRES_UP_V39).await?;
    apply_postgres(pool, &applied, 40, POSTGRES_UP_V40).await?;
    apply_postgres(pool, &applied, 41, POSTGRES_UP_V41).await?;
    apply_postgres(pool, &applied, 42, POSTGRES_UP_V42).await?;
    apply_postgres(pool, &applied, 43, POSTGRES_UP_V43).await?;
    apply_postgres(pool, &applied, 44, POSTGRES_UP_V44).await?;
    apply_postgres(pool, &applied, 45, POSTGRES_UP_V45).await?;
    apply_postgres(pool, &applied, 46, POSTGRES_UP_V46).await?;
    apply_postgres(pool, &applied, 47, POSTGRES_UP_V47).await?;
    apply_postgres(pool, &applied, 48, POSTGRES_UP_V48).await?;
    apply_postgres(pool, &applied, 49, POSTGRES_UP_V49).await?;
    apply_postgres(pool, &applied, 50, POSTGRES_UP_V50).await?;
    apply_postgres(pool, &applied, 51, POSTGRES_UP_V51).await?;
    apply_postgres(pool, &applied, 52, POSTGRES_UP_V52).await?;
    apply_postgres(pool, &applied, 53, POSTGRES_UP_V53).await?;
    apply_postgres(pool, &applied, 54, POSTGRES_UP_V54).await?;
    apply_postgres(pool, &applied, 55, POSTGRES_UP_V55).await?;
    apply_postgres(pool, &applied, 56, POSTGRES_UP_V56).await?;
    apply_postgres(pool, &applied, 57, POSTGRES_UP_V57).await?;
    apply_postgres(pool, &applied, 58, POSTGRES_UP_V58).await?;
    apply_postgres(pool, &applied, 59, POSTGRES_UP_V59).await?;
    apply_postgres(pool, &applied, 60, POSTGRES_UP_V60).await?;
    apply_postgres(pool, &applied, 61, POSTGRES_UP_V61).await?;
    apply_postgres(pool, &applied, 62, POSTGRES_UP_V62).await?;
    apply_postgres(pool, &applied, 63, POSTGRES_UP_V63).await?;
    apply_postgres(pool, &applied, 64, POSTGRES_UP_V64).await?;
    apply_postgres(pool, &applied, 65, POSTGRES_UP_V65).await?;
    apply_postgres(pool, &applied, 66, POSTGRES_UP_V66).await?;
    apply_postgres(pool, &applied, 67, POSTGRES_UP_V67).await?;
    apply_postgres(pool, &applied, 68, POSTGRES_UP_V68).await?;
    apply_postgres(pool, &applied, 69, POSTGRES_UP_V69).await?;
    apply_postgres(pool, &applied, 70, POSTGRES_UP_V70).await?;
    apply_postgres(pool, &applied, 71, POSTGRES_UP_V71).await?;
    apply_postgres(pool, &applied, 72, POSTGRES_UP_V72).await?;
    apply_postgres(pool, &applied, 73, POSTGRES_UP_V73).await?;
    apply_postgres(pool, &applied, 74, POSTGRES_UP_V74).await?;
    apply_postgres(pool, &applied, 75, POSTGRES_UP_V75).await?;
    apply_postgres(pool, &applied, 76, POSTGRES_UP_V76).await?;
    apply_postgres(pool, &applied, 77, POSTGRES_UP_V77).await?;
    apply_postgres(pool, &applied, 78, POSTGRES_UP_V78).await?;
    apply_postgres(pool, &applied, 79, POSTGRES_UP_V79).await?;
    apply_postgres(pool, &applied, 80, POSTGRES_UP_V80).await?;
    apply_postgres(pool, &applied, 81, POSTGRES_UP_V81).await?;
    apply_postgres(pool, &applied, 82, POSTGRES_UP_V82).await?;
    apply_postgres(pool, &applied, 83, POSTGRES_UP_V83).await?;
    apply_postgres(pool, &applied, 84, POSTGRES_UP_V84).await?;
    apply_postgres(pool, &applied, 85, POSTGRES_UP_V85).await?;
    apply_postgres(pool, &applied, 86, POSTGRES_UP_V86).await?;
    apply_postgres(pool, &applied, 87, POSTGRES_UP_V87).await?;
    apply_postgres(pool, &applied, 88, POSTGRES_UP_V88).await?;
    apply_postgres(pool, &applied, 89, POSTGRES_UP_V89).await?;
    apply_postgres(pool, &applied, 90, POSTGRES_UP_V90).await?;
    apply_postgres(pool, &applied, 91, POSTGRES_UP_V91).await?;
    crate::postgres::events::backfill_chain(pool).await?;
    apply_postgres(pool, &applied, 92, POSTGRES_UP_V92).await?;
    apply_postgres(pool, &applied, 93, POSTGRES_UP_V93).await?;
    apply_postgres(pool, &applied, 94, POSTGRES_UP_V94).await?;
    apply_postgres(pool, &applied, 95, POSTGRES_UP_V95).await?;
    apply_postgres(pool, &applied, 96, POSTGRES_UP_V96).await?;
    apply_postgres(pool, &applied, 97, POSTGRES_UP_V97).await?;
    apply_postgres(pool, &applied, 98, POSTGRES_UP_V98).await?;
    apply_postgres(pool, &applied, 99, POSTGRES_UP_V99).await?;
    apply_postgres(pool, &applied, 100, POSTGRES_UP_V100).await?;
    apply_postgres(pool, &applied, 101, POSTGRES_UP_V101).await?;
    apply_postgres(pool, &applied, 102, POSTGRES_UP_V102).await?;
    apply_postgres(pool, &applied, 103, POSTGRES_UP_V103).await?;
    apply_postgres(pool, &applied, 104, POSTGRES_UP_V104).await?;
    apply_postgres(pool, &applied, 105, POSTGRES_UP_V105).await?;
    apply_postgres(pool, &applied, 106, POSTGRES_UP_V106).await?;
    apply_postgres(pool, &applied, 107, POSTGRES_UP_V107).await?;
    apply_postgres(pool, &applied, 108, POSTGRES_UP_V108).await?;
    apply_postgres(pool, &applied, 109, POSTGRES_UP_V109).await?;
    apply_postgres(pool, &applied, 110, POSTGRES_UP_V110).await?;
    apply_postgres(pool, &applied, 111, POSTGRES_UP_V111).await?;
    apply_postgres(pool, &applied, 112, POSTGRES_UP_V112).await?;
    apply_postgres(pool, &applied, 113, POSTGRES_UP_V113).await?;
    apply_postgres(pool, &applied, 114, POSTGRES_UP_V114).await?;
    apply_postgres(pool, &applied, 115, POSTGRES_UP_V115).await?;
    apply_postgres(pool, &applied, 116, POSTGRES_UP_V116).await?;
    apply_postgres(pool, &applied, 117, POSTGRES_UP_V117).await?;
    apply_postgres(pool, &applied, 118, POSTGRES_UP_V118).await?;
    apply_postgres(pool, &applied, 119, POSTGRES_UP_V119).await?;
    apply_postgres(pool, &applied, 120, POSTGRES_UP_V120).await?;
    apply_postgres(pool, &applied, 121, POSTGRES_UP_V121).await?;
    apply_postgres(pool, &applied, 122, POSTGRES_UP_V122).await?;
    apply_postgres(pool, &applied, 123, POSTGRES_UP_V123).await?;
    apply_postgres(pool, &applied, 124, POSTGRES_UP_V124).await?;
    // 125 and 133 are unused; 129 is #1142's session-from-token migration.
    apply_postgres(pool, &applied, 126, POSTGRES_UP_V126).await?;
    apply_postgres(pool, &applied, 127, POSTGRES_UP_V127).await?;
    apply_postgres(pool, &applied, 128, POSTGRES_UP_V128).await?;
    apply_postgres(pool, &applied, 129, POSTGRES_UP_V129).await?;
    apply_postgres(pool, &applied, 130, POSTGRES_UP_V130).await?;
    apply_postgres(pool, &applied, 131, POSTGRES_UP_V131).await?;
    apply_postgres(pool, &applied, 132, POSTGRES_UP_V132).await?;
    apply_postgres(pool, &applied, 134, POSTGRES_UP_V134).await?;
    apply_postgres(pool, &applied, 135, POSTGRES_UP_V135).await?;
    apply_postgres(pool, &applied, 136, POSTGRES_UP_V136).await?;
    apply_postgres(pool, &applied, 137, POSTGRES_UP_V137).await?;
    apply_postgres(pool, &applied, 138, POSTGRES_UP_V138).await?;
    apply_postgres(pool, &applied, 139, POSTGRES_UP_V139).await?;
    apply_postgres(pool, &applied, 140, POSTGRES_UP_V140).await?;
    apply_postgres(pool, &applied, 141, POSTGRES_UP_V141).await?;
    apply_postgres(pool, &applied, 142, POSTGRES_UP_V142).await?;
    apply_postgres(pool, &applied, 143, POSTGRES_UP_V143).await?;
    apply_postgres(pool, &applied, 144, POSTGRES_UP_V144).await?;
    apply_postgres(pool, &applied, 145, POSTGRES_UP_V145).await?;
    apply_postgres(pool, &applied, 146, POSTGRES_UP_V146).await?;
    apply_postgres(pool, &applied, 147, POSTGRES_UP_V147).await?;
    apply_postgres(pool, &applied, 148, POSTGRES_UP_V148).await?;
    apply_postgres(pool, &applied, 149, POSTGRES_UP_V149).await?;
    apply_postgres(pool, &applied, 150, POSTGRES_UP_V150).await?;
    apply_postgres(pool, &applied, 151, POSTGRES_UP_V151).await?;
    apply_postgres(pool, &applied, 152, POSTGRES_UP_V152).await?;
    apply_postgres(pool, &applied, 153, POSTGRES_UP_V153).await?;
    apply_postgres(pool, &applied, 160, POSTGRES_UP_V160).await?;
    apply_postgres(pool, &applied, 161, POSTGRES_UP_V161).await?;
    apply_postgres(pool, &applied, 162, POSTGRES_UP_V162).await?;
    apply_postgres(pool, &applied, 163, POSTGRES_UP_V163).await?;
    apply_postgres(pool, &applied, 164, POSTGRES_UP_V164).await?;
    apply_postgres(pool, &applied, 165, POSTGRES_UP_V165).await?;
    apply_postgres(pool, &applied, 166, POSTGRES_UP_V166).await?;
    apply_postgres(pool, &applied, 167, POSTGRES_UP_V167).await?;
    apply_postgres(pool, &applied, 168, POSTGRES_UP_V168).await?;
    apply_postgres(pool, &applied, 170, POSTGRES_UP_V170).await?;
    apply_postgres(pool, &applied, 171, POSTGRES_UP_V171).await?;
    apply_postgres(pool, &applied, 172, POSTGRES_UP_V172).await?;
    apply_postgres(pool, &applied, 173, POSTGRES_UP_V173).await?;
    apply_postgres(pool, &applied, 174, POSTGRES_UP_V174).await?;
    apply_postgres(pool, &applied, 175, POSTGRES_UP_V175).await?;
    apply_postgres(pool, &applied, 176, POSTGRES_UP_V176).await?;
    apply_postgres(pool, &applied, 177, POSTGRES_UP_V177).await?;
    apply_postgres(pool, &applied, 178, POSTGRES_UP_V178).await?;
    apply_postgres(pool, &applied, 180, POSTGRES_UP_V180).await?;
    apply_postgres(pool, &applied, 181, POSTGRES_UP_V181).await?;
    apply_postgres(pool, &applied, 182, POSTGRES_UP_V182).await?;
    apply_postgres(pool, &applied, 183, POSTGRES_UP_V183).await?;
    apply_postgres(pool, &applied, 184, POSTGRES_UP_V184).await?;
    apply_postgres(pool, &applied, 185, POSTGRES_UP_V185).await?;
    apply_postgres(pool, &applied, 186, POSTGRES_UP_V186).await?;
    apply_postgres(pool, &applied, 187, POSTGRES_UP_V187).await?;
    apply_postgres(pool, &applied, 188, POSTGRES_UP_V188).await?;
    apply_postgres(pool, &applied, 189, POSTGRES_UP_V189).await?;
    apply_postgres(pool, &applied, 190, POSTGRES_UP_V190).await?;
    apply_postgres(pool, &applied, 191, POSTGRES_UP_V191).await?;
    apply_postgres(pool, &applied, 192, POSTGRES_UP_V192).await?;
    apply_postgres(pool, &applied, 193, POSTGRES_UP_V193).await?;
    // 0194 (the change-pull workspace key) lands in its own pull request;
    // 0195 does not depend on it, so either may apply first.
    apply_postgres(pool, &applied, 195, POSTGRES_UP_V195).await?;
    // The partitioned tables need their months ready before the first write,
    // or rows land in DEFAULT until the first retention sweep.
    crate::postgres::partitions::maintain(pool, chrono::Utc::now()).await?;
    Ok(())
}

/// Apply all SQLite migrations to the pool, idempotently.
pub async fn run_sqlite_migrations(pool: &SqlitePool) -> Result<(), StoreError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS maidan_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(pool)
    .await?;
    let applied = applied_versions_sqlite(pool).await?;

    apply_sqlite(pool, &applied, 1, SQLITE_UP_V1).await?;
    apply_sqlite(pool, &applied, 2, SQLITE_UP_V2).await?;
    apply_sqlite(pool, &applied, 3, SQLITE_UP_V3).await?;
    apply_sqlite(pool, &applied, 4, SQLITE_UP_V4).await?;
    apply_sqlite(pool, &applied, 5, SQLITE_UP_V5).await?;
    apply_sqlite(pool, &applied, 6, SQLITE_UP_V6).await?;
    apply_sqlite(pool, &applied, 7, SQLITE_UP_V7).await?;
    apply_sqlite(pool, &applied, 8, SQLITE_UP_V8).await?;
    apply_sqlite(pool, &applied, 9, SQLITE_UP_V9).await?;
    apply_sqlite(pool, &applied, 10, SQLITE_UP_V10).await?;
    apply_sqlite(pool, &applied, 11, SQLITE_UP_V11).await?;
    apply_sqlite(pool, &applied, 12, SQLITE_UP_V12).await?;
    apply_sqlite(pool, &applied, 13, SQLITE_UP_V13).await?;
    apply_sqlite(pool, &applied, 14, SQLITE_UP_V14).await?;
    apply_sqlite(pool, &applied, 15, SQLITE_UP_V15).await?;
    apply_sqlite(pool, &applied, 16, SQLITE_UP_V16).await?;
    apply_sqlite(pool, &applied, 17, SQLITE_UP_V17).await?;
    apply_sqlite(pool, &applied, 18, SQLITE_UP_V18).await?;
    apply_sqlite(pool, &applied, 19, SQLITE_UP_V19).await?;
    apply_sqlite(pool, &applied, 20, SQLITE_UP_V20).await?;
    apply_sqlite(pool, &applied, 21, SQLITE_UP_V21).await?;
    apply_sqlite(pool, &applied, 22, SQLITE_UP_V22).await?;
    apply_sqlite(pool, &applied, 23, SQLITE_UP_V23).await?;
    apply_sqlite(pool, &applied, 24, SQLITE_UP_V24).await?;
    apply_sqlite(pool, &applied, 25, SQLITE_UP_V25).await?;
    apply_sqlite(pool, &applied, 26, SQLITE_UP_V26).await?;
    apply_sqlite(pool, &applied, 27, SQLITE_UP_V27).await?;
    apply_sqlite(pool, &applied, 28, SQLITE_UP_V28).await?;
    apply_sqlite(pool, &applied, 29, SQLITE_UP_V29).await?;
    apply_sqlite(pool, &applied, 30, SQLITE_UP_V30).await?;
    apply_sqlite(pool, &applied, 31, SQLITE_UP_V31).await?;
    apply_sqlite(pool, &applied, 32, SQLITE_UP_V32).await?;
    apply_sqlite(pool, &applied, 33, SQLITE_UP_V33).await?;
    apply_sqlite(pool, &applied, 34, SQLITE_UP_V34).await?;
    apply_sqlite(pool, &applied, 35, SQLITE_UP_V35).await?;
    apply_sqlite(pool, &applied, 36, SQLITE_UP_V36).await?;
    apply_sqlite(pool, &applied, 37, SQLITE_UP_V37).await?;
    apply_sqlite(pool, &applied, 38, SQLITE_UP_V38).await?;
    apply_sqlite(pool, &applied, 39, SQLITE_UP_V39).await?;
    apply_sqlite(pool, &applied, 40, SQLITE_UP_V40).await?;
    apply_sqlite(pool, &applied, 41, SQLITE_UP_V41).await?;
    apply_sqlite(pool, &applied, 42, SQLITE_UP_V42).await?;
    apply_sqlite(pool, &applied, 43, SQLITE_UP_V43).await?;
    apply_sqlite(pool, &applied, 44, SQLITE_UP_V44).await?;
    apply_sqlite(pool, &applied, 45, SQLITE_UP_V45).await?;
    apply_sqlite(pool, &applied, 46, SQLITE_UP_V46).await?;
    apply_sqlite(pool, &applied, 47, SQLITE_UP_V47).await?;
    apply_sqlite(pool, &applied, 48, SQLITE_UP_V48).await?;
    apply_sqlite(pool, &applied, 49, SQLITE_UP_V49).await?;
    apply_sqlite(pool, &applied, 50, SQLITE_UP_V50).await?;
    apply_sqlite(pool, &applied, 51, SQLITE_UP_V51).await?;
    apply_sqlite(pool, &applied, 52, SQLITE_UP_V52).await?;
    apply_sqlite(pool, &applied, 53, SQLITE_UP_V53).await?;
    apply_sqlite(pool, &applied, 54, SQLITE_UP_V54).await?;
    apply_sqlite(pool, &applied, 55, SQLITE_UP_V55).await?;
    apply_sqlite(pool, &applied, 56, SQLITE_UP_V56).await?;
    apply_sqlite(pool, &applied, 57, SQLITE_UP_V57).await?;
    apply_sqlite(pool, &applied, 58, SQLITE_UP_V58).await?;
    apply_sqlite(pool, &applied, 59, SQLITE_UP_V59).await?;
    apply_sqlite(pool, &applied, 60, SQLITE_UP_V60).await?;
    apply_sqlite(pool, &applied, 61, SQLITE_UP_V61).await?;
    apply_sqlite(pool, &applied, 62, SQLITE_UP_V62).await?;
    apply_sqlite(pool, &applied, 63, SQLITE_UP_V63).await?;
    apply_sqlite(pool, &applied, 64, SQLITE_UP_V64).await?;
    apply_sqlite(pool, &applied, 65, SQLITE_UP_V65).await?;
    apply_sqlite(pool, &applied, 66, SQLITE_UP_V66).await?;
    apply_sqlite(pool, &applied, 67, SQLITE_UP_V67).await?;
    apply_sqlite(pool, &applied, 68, SQLITE_UP_V68).await?;
    apply_sqlite(pool, &applied, 69, SQLITE_UP_V69).await?;
    apply_sqlite(pool, &applied, 70, SQLITE_UP_V70).await?;
    apply_sqlite(pool, &applied, 71, SQLITE_UP_V71).await?;
    apply_sqlite(pool, &applied, 72, SQLITE_UP_V72).await?;
    apply_sqlite(pool, &applied, 73, SQLITE_UP_V73).await?;
    apply_sqlite(pool, &applied, 74, SQLITE_UP_V74).await?;
    apply_sqlite(pool, &applied, 75, SQLITE_UP_V75).await?;
    apply_sqlite(pool, &applied, 76, SQLITE_UP_V76).await?;
    apply_sqlite(pool, &applied, 77, SQLITE_UP_V77).await?;
    apply_sqlite(pool, &applied, 78, SQLITE_UP_V78).await?;
    apply_sqlite(pool, &applied, 79, SQLITE_UP_V79).await?;
    apply_sqlite(pool, &applied, 80, SQLITE_UP_V80).await?;
    apply_sqlite(pool, &applied, 81, SQLITE_UP_V81).await?;
    apply_sqlite(pool, &applied, 82, SQLITE_UP_V82).await?;
    apply_sqlite(pool, &applied, 83, SQLITE_UP_V83).await?;
    apply_sqlite(pool, &applied, 84, SQLITE_UP_V84).await?;
    apply_sqlite(pool, &applied, 85, SQLITE_UP_V85).await?;
    apply_sqlite(pool, &applied, 86, SQLITE_UP_V86).await?;
    apply_sqlite(pool, &applied, 87, SQLITE_UP_V87).await?;
    apply_sqlite(pool, &applied, 88, SQLITE_UP_V88).await?;
    apply_sqlite(pool, &applied, 89, SQLITE_UP_V89).await?;
    apply_sqlite(pool, &applied, 90, SQLITE_UP_V90).await?;
    crate::sqlite::events::backfill_chain(pool).await?;
    apply_sqlite(pool, &applied, 91, SQLITE_UP_V91).await?;
    apply_sqlite(pool, &applied, 92, SQLITE_UP_V92).await?;
    apply_sqlite(pool, &applied, 93, SQLITE_UP_V93).await?;
    apply_sqlite(pool, &applied, 94, SQLITE_UP_V94).await?;
    apply_sqlite(pool, &applied, 95, SQLITE_UP_V95).await?;
    apply_sqlite(pool, &applied, 96, SQLITE_UP_V96).await?;
    apply_sqlite(pool, &applied, 97, SQLITE_UP_V97).await?;
    apply_sqlite(pool, &applied, 98, SQLITE_UP_V98).await?;
    apply_sqlite(pool, &applied, 99, SQLITE_UP_V99).await?;
    apply_sqlite(pool, &applied, 100, SQLITE_UP_V100).await?;
    apply_sqlite(pool, &applied, 101, SQLITE_UP_V101).await?;
    apply_sqlite(pool, &applied, 102, SQLITE_UP_V102).await?;
    apply_sqlite(pool, &applied, 103, SQLITE_UP_V103).await?;
    apply_sqlite(pool, &applied, 104, SQLITE_UP_V104).await?;
    apply_sqlite(pool, &applied, 105, SQLITE_UP_V105).await?;
    apply_sqlite(pool, &applied, 106, SQLITE_UP_V106).await?;
    apply_sqlite(pool, &applied, 107, SQLITE_UP_V107).await?;
    apply_sqlite(pool, &applied, 108, SQLITE_UP_V108).await?;
    apply_sqlite(pool, &applied, 109, SQLITE_UP_V109).await?;
    apply_sqlite(pool, &applied, 110, SQLITE_UP_V110).await?;
    apply_sqlite(pool, &applied, 111, SQLITE_UP_V111).await?;
    apply_sqlite(pool, &applied, 112, SQLITE_UP_V112).await?;
    apply_sqlite(pool, &applied, 113, SQLITE_UP_V113).await?;
    apply_sqlite(pool, &applied, 114, SQLITE_UP_V114).await?;
    apply_sqlite(pool, &applied, 115, SQLITE_UP_V115).await?;
    apply_sqlite(pool, &applied, 116, SQLITE_UP_V116).await?;
    apply_sqlite(pool, &applied, 117, SQLITE_UP_V117).await?;
    apply_sqlite(pool, &applied, 118, SQLITE_UP_V118).await?;
    apply_sqlite(pool, &applied, 119, SQLITE_UP_V119).await?;
    apply_sqlite(pool, &applied, 120, SQLITE_UP_V120).await?;
    apply_sqlite(pool, &applied, 121, SQLITE_UP_V121).await?;
    apply_sqlite(pool, &applied, 122, SQLITE_UP_V122).await?;
    apply_sqlite(pool, &applied, 123, SQLITE_UP_V123).await?;
    apply_sqlite(pool, &applied, 124, SQLITE_UP_V124).await?;
    // 125 and 133 are unused; 129 is #1142's session-from-token migration.
    apply_sqlite(pool, &applied, 126, SQLITE_UP_V126).await?;
    apply_sqlite(pool, &applied, 127, SQLITE_UP_V127).await?;
    apply_sqlite(pool, &applied, 128, SQLITE_UP_V128).await?;
    apply_sqlite(pool, &applied, 129, SQLITE_UP_V129).await?;
    apply_sqlite(pool, &applied, 130, SQLITE_UP_V130).await?;
    apply_sqlite(pool, &applied, 131, SQLITE_UP_V131).await?;
    apply_sqlite(pool, &applied, 132, SQLITE_UP_V132).await?;
    apply_sqlite(pool, &applied, 134, SQLITE_UP_V134).await?;
    apply_sqlite(pool, &applied, 135, SQLITE_UP_V135).await?;
    apply_sqlite(pool, &applied, 136, SQLITE_UP_V136).await?;
    apply_sqlite(pool, &applied, 137, SQLITE_UP_V137).await?;
    apply_sqlite(pool, &applied, 138, SQLITE_UP_V138).await?;
    apply_sqlite(pool, &applied, 139, SQLITE_UP_V139).await?;
    apply_sqlite(pool, &applied, 140, SQLITE_UP_V140).await?;
    apply_sqlite(pool, &applied, 141, SQLITE_UP_V141).await?;
    apply_sqlite(pool, &applied, 142, SQLITE_UP_V142).await?;
    apply_sqlite(pool, &applied, 143, SQLITE_UP_V143).await?;
    apply_sqlite(pool, &applied, 144, SQLITE_UP_V144).await?;
    apply_sqlite(pool, &applied, 145, SQLITE_UP_V145).await?;
    apply_sqlite(pool, &applied, 146, SQLITE_UP_V146).await?;
    apply_sqlite(pool, &applied, 147, SQLITE_UP_V147).await?;
    apply_sqlite(pool, &applied, 148, SQLITE_UP_V148).await?;
    apply_sqlite(pool, &applied, 149, SQLITE_UP_V149).await?;
    apply_sqlite(pool, &applied, 150, SQLITE_UP_V150).await?;
    apply_sqlite(pool, &applied, 151, SQLITE_UP_V151).await?;
    apply_sqlite(pool, &applied, 152, SQLITE_UP_V152).await?;
    apply_sqlite(pool, &applied, 153, SQLITE_UP_V153).await?;
    apply_sqlite(pool, &applied, 160, SQLITE_UP_V160).await?;
    apply_sqlite(pool, &applied, 161, SQLITE_UP_V161).await?;
    apply_sqlite(pool, &applied, 163, SQLITE_UP_V163).await?;
    apply_sqlite(pool, &applied, 164, SQLITE_UP_V164).await?;
    apply_sqlite(pool, &applied, 167, SQLITE_UP_V167).await?;
    apply_sqlite(pool, &applied, 168, SQLITE_UP_V168).await?;
    apply_sqlite(pool, &applied, 170, SQLITE_UP_V170).await?;
    apply_sqlite(pool, &applied, 171, SQLITE_UP_V171).await?;
    apply_sqlite(pool, &applied, 172, SQLITE_UP_V172).await?;
    apply_sqlite(pool, &applied, 173, SQLITE_UP_V173).await?;
    apply_sqlite(pool, &applied, 174, SQLITE_UP_V174).await?;
    apply_sqlite(pool, &applied, 175, SQLITE_UP_V175).await?;
    apply_sqlite(pool, &applied, 176, SQLITE_UP_V176).await?;
    apply_sqlite(pool, &applied, 177, SQLITE_UP_V177).await?;
    apply_sqlite(pool, &applied, 178, SQLITE_UP_V178).await?;
    apply_sqlite(pool, &applied, 180, SQLITE_UP_V180).await?;
    apply_sqlite(pool, &applied, 181, SQLITE_UP_V181).await?;
    apply_sqlite(pool, &applied, 182, SQLITE_UP_V182).await?;
    apply_sqlite(pool, &applied, 183, SQLITE_UP_V183).await?;
    apply_sqlite(pool, &applied, 184, SQLITE_UP_V184).await?;
    apply_sqlite(pool, &applied, 185, SQLITE_UP_V185).await?;
    apply_sqlite(pool, &applied, 186, SQLITE_UP_V186).await?;
    apply_sqlite(pool, &applied, 187, SQLITE_UP_V187).await?;
    apply_sqlite(pool, &applied, 188, SQLITE_UP_V188).await?;
    apply_sqlite(pool, &applied, 189, SQLITE_UP_V189).await?;
    apply_sqlite(pool, &applied, 190, SQLITE_UP_V190).await?;
    apply_sqlite(pool, &applied, 191, SQLITE_UP_V191).await?;
    apply_sqlite(pool, &applied, 192, SQLITE_UP_V192).await?;
    apply_sqlite(pool, &applied, 193, SQLITE_UP_V193).await?;
    // 0195 does not depend on 0194, which lands in its own pull request.
    apply_sqlite(pool, &applied, 195, SQLITE_UP_V195).await?;
    Ok(())
}

async fn applied_versions_postgres(pool: &PgPool) -> Result<HashSet<i64>, StoreError> {
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT version FROM maidan_migrations")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(v,)| v).collect())
}

async fn applied_versions_sqlite(pool: &SqlitePool) -> Result<HashSet<i64>, StoreError> {
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT version FROM maidan_migrations")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(v,)| v).collect())
}

async fn apply_postgres(
    pool: &PgPool,
    applied: &HashSet<i64>,
    version: i64,
    sql: &str,
) -> Result<(), StoreError> {
    if applied.contains(&version) {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    sqlx::raw_sql(sql).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO maidan_migrations (version) VALUES ($1)")
        .bind(version)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(version, "applied postgres migration");
    Ok(())
}

async fn apply_sqlite(
    pool: &SqlitePool,
    applied: &HashSet<i64>,
    version: i64,
    sql: &str,
) -> Result<(), StoreError> {
    if applied.contains(&version) {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    sqlx::raw_sql(sql).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO maidan_migrations (version) VALUES (?)")
        .bind(version)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(version, "applied sqlite migration");
    Ok(())
}
