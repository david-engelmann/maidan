//! Finding a thread from where it is referenced outside Maidan: a Slack
//! message (or a linked Slack channel), or a GitHub pull request. Checked
//! here once for `GET /workspaces/{id}/thread-lookup` and the MCP
//! `find_thread`, so both refuse the same requests the same way.

use serde::{Deserialize, Serialize};

use crate::{is_slack_ts, normalize_repo};

/// A lookup as a caller gives it: a Slack channel id with or without a
/// message `ts`, or a repository and pull request number.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadLookup {
    #[serde(default)]
    pub slack_channel: Option<String>,
    #[serde(default)]
    pub slack_ts: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub pr_number: Option<i64>,
}

/// A checked [`ThreadLookup`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalThreadRef {
    /// With `ts`, the thread whose result was posted as that message or into
    /// that Slack thread. Without it, the thread the channel is linked to.
    Slack { channel: String, ts: Option<String> },
    /// The thread tracking the pull request, linked to it, or delivering its
    /// result to it. `repo` is lowercase `owner/name`.
    Pull { repo: String, number: i64 },
}

impl ThreadLookup {
    pub fn check(&self) -> Result<ExternalThreadRef, String> {
        let slack = self.slack_channel.is_some() || self.slack_ts.is_some();
        let pull = self.repo.is_some() || self.pr_number.is_some();
        match (slack, pull) {
            (true, true) => Err("give a Slack reference or a pull request, not both".into()),
            (false, false) => {
                Err("give slack_channel (and slack_ts), or repo and pr_number".into())
            }
            (true, false) => {
                let channel = self
                    .slack_channel
                    .as_deref()
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .ok_or("slack_ts needs slack_channel")?;
                if channel.starts_with('#') || channel.contains('/') {
                    return Err(format!(
                        "slack_channel `{channel}` is not a Slack channel id such as C0123ABCD"
                    ));
                }
                if let Some(ts) = self.slack_ts.as_deref().filter(|ts| !is_slack_ts(ts)) {
                    return Err(format!(
                        "slack_ts `{ts}` is not a Slack message ts such as 1700000000.000100"
                    ));
                }
                Ok(ExternalThreadRef::Slack {
                    channel: channel.to_string(),
                    ts: self.slack_ts.clone(),
                })
            }
            (false, true) => {
                let (Some(repo), Some(number)) = (self.repo.as_deref(), self.pr_number) else {
                    return Err("repo and pr_number go together".into());
                };
                if number < 1 {
                    return Err("pr_number is at least 1".into());
                }
                Ok(ExternalThreadRef::Pull {
                    repo: normalize_repo(repo)?,
                    number,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(
        slack: (Option<&str>, Option<&str>),
        pull: (Option<&str>, Option<i64>),
    ) -> ThreadLookup {
        ThreadLookup {
            slack_channel: slack.0.map(Into::into),
            slack_ts: slack.1.map(Into::into),
            repo: pull.0.map(Into::into),
            pr_number: pull.1,
        }
    }

    #[test]
    fn a_lookup_names_one_reference_and_names_it_well() {
        assert_eq!(
            lookup((Some("C0123"), Some("1700000000.000100")), (None, None)).check(),
            Ok(ExternalThreadRef::Slack {
                channel: "C0123".into(),
                ts: Some("1700000000.000100".into())
            })
        );
        assert_eq!(
            lookup(
                (None, None),
                (Some("https://github.com/Example/App"), Some(7))
            )
            .check(),
            Ok(ExternalThreadRef::Pull {
                repo: "example/app".into(),
                number: 7
            })
        );
        for bad in [
            lookup((None, None), (None, None)),
            lookup((Some("C0123"), None), (Some("example/app"), Some(7))),
            lookup((None, Some("1700000000.000100")), (None, None)),
            lookup((Some("#general"), None), (None, None)),
            lookup((Some("C0123"), Some("yesterday")), (None, None)),
            lookup((None, None), (Some("example/app"), None)),
            lookup((None, None), (Some("example/app"), Some(0))),
        ] {
            assert!(bad.check().is_err(), "{bad:?}");
        }
    }
}
