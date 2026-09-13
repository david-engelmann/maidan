//! The waiter-result contract (Cluster 379.2) — a **tolerant reader** for the
//! envelope an external result producer writes with `set_thread_result`.
//!
//! The result is opaque JSON owned by the producer. Maidan parses only the fields
//! it *routes on* and ignores everything else, so a producer adding a field never
//! breaks delivery. The grammar is pinned in `docs/Result Delivery.md` and frozen
//! at [`WAITER_RESULT_SCHEMA`]; the authoritative fixture is committed at
//! `tests/fixtures/pi_waiter_result_v1.json`, so a producer-side grammar change
//! breaks a test in this crate rather than a delivery in production.
//!
//! **Tolerance has a floor.** `schema` is the discriminator: an unrecognized one
//! means no delivery is attempted at all, because routing on an envelope we do
//! not understand is how you deliver the wrong bytes to the wrong place. Past
//! that, missing optional fields degrade rather than fail.

use serde::Serialize;
use serde_json::Value;

use crate::egress::EgressTarget;

/// The frozen envelope discriminator. A different value is inert, not an error.
pub const WAITER_RESULT_SCHEMA: &str = "pi.waiter.result/1";

/// The only `status` that delivers the producer's own bytes. Any other value
/// gets a short Maidan-authored failure notice built from `status` alone —
/// never silence, and never rendered as a clean pass.
pub const STATUS_REVIEWED: &str = "reviewed";

/// One entry of the producer's `deliver_to` routing list.
///
/// [`Self::Unknown`] is the load-bearing variant: rule 2 of the pinned grammar
/// says an unknown `surface` is **skipped with a recorded warning, never an
/// error**, so a newer producer naming a surface this build cannot reach stays
/// backward-compatible. It carries the surface string so the skip can say which
/// one it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "surface", rename_all = "snake_case")]
pub enum DeliverTarget {
    /// `repo` is `owner/name`; `pr` is an issue **or** PR number — GitHub shares
    /// one namespace, so a PR comments exactly like an issue.
    Github {
        repo: String,
        pr: i64,
    },
    /// A channel **id** (`C…`/`G…`), never a `#name`: a name is mutable, and the
    /// channel it points at can change under a delivery.
    Slack {
        channel: String,
    },
    Unknown(String),
}

impl DeliverTarget {
    /// The surface name as the producer wrote it.
    pub fn surface(&self) -> &str {
        match self {
            Self::Github { .. } => "github",
            Self::Slack { .. } => "slack",
            Self::Unknown(surface) => surface,
        }
    }

    /// Project a producer-selected target onto the delivery target the egress
    /// queue speaks (Cluster 377).
    ///
    /// `None` for a surface this build does not know, and for a target whose
    /// per-surface detail is missing or unusable — both of which are a *skip*
    /// with a recorded reason, not a failure. This is where the producer's
    /// vocabulary stops and Maidan's transport begins.
    pub fn to_egress_target(&self) -> Option<EgressTarget> {
        match self {
            Self::Github { repo, pr } => {
                let (owner, name) = repo.split_once('/')?;
                (!owner.is_empty() && !name.is_empty() && !repo.contains('#') && *pr > 0).then(
                    || EgressTarget::Github {
                        repo: repo.clone(),
                        issue_number: *pr,
                    },
                )
            }
            Self::Slack { channel } => {
                (!channel.is_empty() && !channel.starts_with('#')).then(|| EgressTarget::Slack {
                    channel_id: channel.clone(),
                })
            }
            Self::Unknown(_) => None,
        }
    }

    /// The `(surface, selector)` pair a skip is recorded under when this target
    /// cannot be projected onto an [`EgressTarget`].
    ///
    /// Known surfaces keep the producer-supplied detail so the skip is
    /// addressable (`github` / `owner/name#123`, `slack` / `#general`). An
    /// unknown surface records under its name with an empty selector — there
    /// is no delivery grain to store, but the row still has to exist, because
    /// "we skipped your target" and "we lost it" are different answers.
    pub fn skip_fingerprint(&self) -> (String, String) {
        match self {
            Self::Github { repo, pr } => ("github".into(), format!("{repo}#{pr}")),
            Self::Slack { channel } => ("slack".into(), channel.clone()),
            Self::Unknown(surface) => (surface.clone(), String::new()),
        }
    }
}

/// The routable projection of a producer's result envelope. Everything the
/// producer carries that Maidan does not route on — `findings`, `corroboration`,
/// `per_seat`, `run_id`, `cost_usd`, `head_sha`, … — stays in the stored result
/// and is deliberately absent here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaiterResult {
    /// Which producer shape this is, e.g. `pi.review.result/1`. A **namespaced
    /// string, not an enum** — a closed enum would need editing every time a
    /// waiter product ships a new result kind.
    pub result_kind: String,
    pub status: String,
    /// Absent or empty is **valid and normal**: thread-only, delivered nowhere.
    pub deliver_to: Vec<DeliverTarget>,
    /// The GitHub delivery body — producer-authored trusted markdown.
    pub rendered: Option<String>,
    /// One line. The Slack body and the notification title.
    pub summary: Option<String>,
    /// A backlink appended to every delivery.
    pub view_in_pi: Option<String>,
    /// A human back-reference echoed into the delivered body. **A string**
    /// (`owner/name#123`) — not to be confused with `deliver_to[].pr`, which is
    /// an integer. Two different fields with the same name and different types.
    pub pr: Option<String>,
}

impl WaiterResult {
    /// Whether this result delivers the producer's own bytes. A non-`reviewed`
    /// status is surfaced as a failure notice instead — never silence, never a
    /// clean pass.
    pub fn is_reviewed(&self) -> bool {
        self.status == STATUS_REVIEWED
    }
}

/// Read the routable fields out of a producer's result.
///
/// `None` when this is not an envelope we recognize — a missing or unexpected
/// `schema`, or a non-object — which means no delivery is attempted rather than
/// a delivery attempted on a guess. Anything past the discriminator degrades
/// instead of failing: an absent `deliver_to` is an empty list, and the optional
/// bodies are `None`.
pub fn parse_waiter_result(value: &Value) -> Option<WaiterResult> {
    let obj = value.as_object()?;
    if obj.get("schema").and_then(Value::as_str)? != WAITER_RESULT_SCHEMA {
        return None;
    }
    // `result_kind` and `status` are the two fields every routing decision reads,
    // so an envelope without them is not routable even though the schema matched.
    let result_kind = obj.get("result_kind").and_then(Value::as_str)?.to_string();
    let status = obj.get("status").and_then(Value::as_str)?.to_string();

    let deliver_to = obj
        .get("deliver_to")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(parse_deliver_target).collect())
        .unwrap_or_default();

    let string_field = |key: &str| {
        obj.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    Some(WaiterResult {
        result_kind,
        status,
        deliver_to,
        rendered: string_field("rendered"),
        summary: string_field("summary"),
        view_in_pi: string_field("view_in_pi"),
        pr: string_field("pr"),
    })
}

/// One `deliver_to` entry. `None` only when the entry has no `surface` string at
/// all — a known surface missing its detail still yields [`DeliverTarget::Unknown`]
/// so the disposition is *recorded* rather than the entry vanishing, which is the
/// difference between "we skipped your target" and "we lost it".
fn parse_deliver_target(entry: &Value) -> Option<DeliverTarget> {
    let obj = entry.as_object()?;
    let surface = obj.get("surface").and_then(Value::as_str)?;
    let unknown = || Some(DeliverTarget::Unknown(surface.to_string()));
    match surface {
        "github" => {
            let Some(repo) = obj.get("repo").and_then(Value::as_str) else {
                return unknown();
            };
            let Some(pr) = obj.get("pr").and_then(Value::as_i64) else {
                return unknown();
            };
            Some(DeliverTarget::Github {
                repo: repo.to_string(),
                pr,
            })
        }
        "slack" => match obj.get("channel").and_then(Value::as_str) {
            Some(channel) => Some(DeliverTarget::Slack {
                channel: channel.to_string(),
            }),
            None => unknown(),
        },
        other => Some(DeliverTarget::Unknown(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unrecognized_envelope_is_inert() {
        // No delivery is attempted on a schema we do not understand, rather than
        // routing on a guess.
        for value in [
            json!({}),
            json!({ "status": "reviewed", "result_kind": "x/1" }),
            json!({ "schema": "pi.waiter.result/2", "result_kind": "x/1", "status": "reviewed" }),
            json!({ "schema": 1, "result_kind": "x/1", "status": "reviewed" }),
            json!("a string, not an envelope"),
            json!([]),
        ] {
            assert_eq!(parse_waiter_result(&value), None, "for {value}");
        }
        // Schema matches, but the two fields every routing decision reads are missing.
        assert_eq!(
            parse_waiter_result(&json!({ "schema": WAITER_RESULT_SCHEMA })),
            None
        );
        assert_eq!(
            parse_waiter_result(
                &json!({ "schema": WAITER_RESULT_SCHEMA, "result_kind": "pi.review.result/1" })
            ),
            None,
            "no status to route on"
        );
    }

    #[test]
    fn an_empty_deliver_to_is_valid_and_normal() {
        // "Delivered nowhere" is a supported outcome, not a misconfiguration —
        // so this parses cleanly rather than erroring.
        let minimal = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "pi.review.result/1",
            "status": "reviewed"
        });
        let parsed = parse_waiter_result(&minimal).expect("parses");
        assert!(parsed.deliver_to.is_empty());
        assert!(parsed.is_reviewed());
        assert_eq!(parsed.rendered, None);
        assert_eq!(parsed.summary, None);

        let explicit = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "pi.review.result/1",
            "status": "reviewed",
            "deliver_to": []
        });
        assert!(parse_waiter_result(&explicit)
            .expect("parses")
            .deliver_to
            .is_empty());
    }

    #[test]
    fn an_unknown_surface_is_recorded_not_dropped_and_not_an_error() {
        let value = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "pi.review.result/1",
            "status": "reviewed",
            "deliver_to": [
                { "surface": "discord", "webhook": "https://x.test/hook" },
                { "surface": "github", "repo": "acme/widgets", "pr": 7 },
                // A known surface missing its detail: still recorded, because a
                // vanished entry and a skipped one are not the same answer.
                { "surface": "github" },
                { "surface": "slack" },
                // No surface at all: nothing to record under.
                { "repo": "acme/widgets", "pr": 8 }
            ]
        });
        let parsed = parse_waiter_result(&value).expect("parses");
        assert_eq!(
            parsed.deliver_to,
            vec![
                DeliverTarget::Unknown("discord".into()),
                DeliverTarget::Github {
                    repo: "acme/widgets".into(),
                    pr: 7
                },
                DeliverTarget::Unknown("github".into()),
                DeliverTarget::Unknown("slack".into()),
            ],
            "one unroutable target never sinks the others — partial delivery is the model"
        );
        assert_eq!(parsed.deliver_to[0].surface(), "discord");
    }

    #[test]
    fn a_target_projects_onto_the_egress_target_the_queue_speaks() {
        assert_eq!(
            DeliverTarget::Github {
                repo: "beatgig/bgv3".into(),
                pr: 3915
            }
            .to_egress_target(),
            Some(EgressTarget::Github {
                repo: "beatgig/bgv3".into(),
                issue_number: 3915
            })
        );
        assert_eq!(
            DeliverTarget::Slack {
                channel: "C0123ABCDEF".into()
            }
            .to_egress_target(),
            Some(EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into()
            })
        );
        assert_eq!(
            DeliverTarget::Unknown("discord".into()).to_egress_target(),
            None
        );
        assert_eq!(
            DeliverTarget::Unknown("discord".into()).skip_fingerprint(),
            ("discord".into(), String::new()),
            "an unknown surface still has a row to record the skip on"
        );
        assert_eq!(
            DeliverTarget::Slack {
                channel: "#general".into()
            }
            .skip_fingerprint(),
            ("slack".into(), "#general".into()),
            "a #name is unusable as a destination but the skip keeps the bytes the producer wrote"
        );
    }

    #[test]
    fn a_malformed_target_does_not_project_so_it_is_skipped_not_delivered() {
        for target in [
            DeliverTarget::Github {
                repo: "bgv3".into(),
                pr: 1,
            },
            DeliverTarget::Github {
                repo: "/bgv3".into(),
                pr: 1,
            },
            DeliverTarget::Github {
                repo: "beatgig/".into(),
                pr: 1,
            },
            // An issue-qualified repo is a producer mistake, not a destination.
            DeliverTarget::Github {
                repo: "beatgig/bgv3#3915".into(),
                pr: 3915,
            },
            DeliverTarget::Github {
                repo: "beatgig/bgv3".into(),
                pr: 0,
            },
            DeliverTarget::Github {
                repo: "beatgig/bgv3".into(),
                pr: -1,
            },
            // A mutable name is not an addressable channel.
            DeliverTarget::Slack {
                channel: "#general".into(),
            },
            DeliverTarget::Slack {
                channel: String::new(),
            },
        ] {
            assert_eq!(
                target.to_egress_target(),
                None,
                "expected {target:?} not to project"
            );
        }
    }

    #[test]
    fn a_non_reviewed_status_parses_but_does_not_deliver_the_producers_bytes() {
        let value = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "pi.review.result/1",
            "status": "failed",
            "deliver_to": [{ "surface": "slack", "channel": "C1" }],
            "rendered": "should not be delivered as a clean pass"
        });
        let parsed = parse_waiter_result(&value).expect("parses");
        assert!(!parsed.is_reviewed());
        assert_eq!(parsed.status, "failed");
        assert_eq!(
            parsed.deliver_to.len(),
            1,
            "the routing list still parses — the failure notice goes to the same targets"
        );
    }

    #[test]
    fn an_empty_string_body_reads_as_absent() {
        // A producer sending "" is telling us it has nothing, not that the body
        // is the empty string; treating them differently would post a blank comment.
        let value = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "pi.review.result/1",
            "status": "reviewed",
            "rendered": "",
            "summary": "",
            "view_in_pi": "",
            "pr": ""
        });
        let parsed = parse_waiter_result(&value).expect("parses");
        assert_eq!(parsed.rendered, None);
        assert_eq!(parsed.summary, None);
        assert_eq!(parsed.view_in_pi, None);
        assert_eq!(parsed.pr, None);
    }

    #[test]
    fn unknown_producer_fields_are_ignored_rather_than_rejected() {
        let value = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "pi.review.result/1",
            "status": "reviewed",
            "summary": "ok",
            "a_field_from_a_future_producer": { "nested": [1, 2, 3] },
            "findings": [{ "severity": "critical" }],
            "cost_usd": 0.65
        });
        let parsed = parse_waiter_result(&value).expect("parses");
        assert_eq!(parsed.summary.as_deref(), Some("ok"));
    }
}
