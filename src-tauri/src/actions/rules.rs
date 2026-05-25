//! Rule engine: pure function over a classified cluster → proposed actions.
//!
//! Kept deliberately small and obvious. Each rule fires independently; we
//! return all matches so the UI can show overlapping suggestions (the user
//! resolves the ambiguity by approving or skipping individually).

use serde::Serialize;

use crate::db::Category;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Archive,
    Trash,
    AddLabel,
    RemoveLabel,
    MarkRead,
    Unsubscribe,
}

impl ActionType {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionType::Archive => "archive",
            ActionType::Trash => "trash",
            ActionType::AddLabel => "add_label",
            ActionType::RemoveLabel => "remove_label",
            ActionType::MarkRead => "mark_read",
            ActionType::Unsubscribe => "unsubscribe",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProposedAction {
    pub action: ActionType,
    pub reason: &'static str,
}

/// Tunable rule thresholds. Defaults match the SPEC.
#[derive(Clone, Debug)]
pub struct RuleConfig {
    /// promotional clusters with messages older than this are archive
    /// candidates.
    pub promotional_archive_age_days: i64,
    /// notification clusters with at least this many members are trash
    /// candidates.
    pub notification_trash_min_members: i64,
    /// only act on classifications at or above this confidence.
    pub min_confidence: f32,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            promotional_archive_age_days: 90,
            notification_trash_min_members: 50,
            min_confidence: 0.6,
        }
    }
}

/// Per-cluster facts the rules need. Computed once by the caller from the DB.
#[derive(Clone, Debug)]
pub struct ClusterFacts {
    pub cluster_key: String,
    pub category: Category,
    pub confidence: f32,
    pub member_count: i64,
    pub oldest_message_age_days: Option<i64>,
    pub has_list_unsubscribe: bool,
}

pub fn propose_actions(facts: &ClusterFacts, cfg: &RuleConfig) -> Vec<ProposedAction> {
    let mut out = Vec::new();
    if facts.confidence < cfg.min_confidence {
        return out; // never propose actions on low-confidence classifications
    }

    match facts.category {
        Category::Promotional => {
            if facts
                .oldest_message_age_days
                .map(|d| d >= cfg.promotional_archive_age_days)
                .unwrap_or(false)
            {
                out.push(ProposedAction {
                    action: ActionType::Archive,
                    reason: "promotional + older than 90 days",
                });
            }
            if facts.has_list_unsubscribe {
                out.push(ProposedAction {
                    action: ActionType::Unsubscribe,
                    reason: "promotional + List-Unsubscribe header present",
                });
            }
        }
        Category::Notification => {
            if facts.member_count >= cfg.notification_trash_min_members {
                out.push(ProposedAction {
                    action: ActionType::Trash,
                    reason: "notification + high volume",
                });
            }
        }
        Category::Newsletter => {
            if facts.has_list_unsubscribe {
                out.push(ProposedAction {
                    action: ActionType::Unsubscribe,
                    reason: "newsletter + List-Unsubscribe header present",
                });
            }
        }
        Category::Social => {
            if facts.member_count >= cfg.notification_trash_min_members {
                out.push(ProposedAction {
                    action: ActionType::Archive,
                    reason: "social + high volume",
                });
            }
        }
        // Security and personal: never propose destructive actions.
        Category::Security | Category::Personal | Category::Work | Category::Unknown => {}
        // Transactional: archive old receipts; never trash.
        Category::Transactional => {
            if facts
                .oldest_message_age_days
                .map(|d| d >= 365)
                .unwrap_or(false)
            {
                out.push(ProposedAction {
                    action: ActionType::Archive,
                    reason: "transactional + older than 1 year",
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_facts(category: Category) -> ClusterFacts {
        ClusterFacts {
            cluster_key: "sender::x@example.com".into(),
            category,
            confidence: 0.95,
            member_count: 10,
            oldest_message_age_days: Some(30),
            has_list_unsubscribe: false,
        }
    }

    #[test]
    fn old_promotional_proposes_archive() {
        let mut f = base_facts(Category::Promotional);
        f.oldest_message_age_days = Some(120);
        let actions = propose_actions(&f, &RuleConfig::default());
        assert!(actions.iter().any(|a| a.action == ActionType::Archive));
    }

    #[test]
    fn recent_promotional_does_not_propose_archive() {
        let f = base_facts(Category::Promotional);
        let actions = propose_actions(&f, &RuleConfig::default());
        assert!(actions.iter().all(|a| a.action != ActionType::Archive));
    }

    #[test]
    fn high_volume_notification_proposes_trash() {
        let mut f = base_facts(Category::Notification);
        f.member_count = 100;
        let actions = propose_actions(&f, &RuleConfig::default());
        assert!(actions.iter().any(|a| a.action == ActionType::Trash));
    }

    #[test]
    fn low_volume_notification_proposes_nothing() {
        let f = base_facts(Category::Notification);
        let actions = propose_actions(&f, &RuleConfig::default());
        assert!(actions.is_empty());
    }

    #[test]
    fn security_is_never_actioned() {
        let mut f = base_facts(Category::Security);
        f.member_count = 1000;
        f.oldest_message_age_days = Some(10_000);
        f.has_list_unsubscribe = true;
        let actions = propose_actions(&f, &RuleConfig::default());
        assert!(actions.is_empty());
    }

    #[test]
    fn personal_is_never_actioned() {
        let f = base_facts(Category::Personal);
        assert!(propose_actions(&f, &RuleConfig::default()).is_empty());
    }

    #[test]
    fn low_confidence_blocks_all_rules() {
        let mut f = base_facts(Category::Promotional);
        f.oldest_message_age_days = Some(120);
        f.confidence = 0.55;
        assert!(propose_actions(&f, &RuleConfig::default()).is_empty());
    }

    #[test]
    fn newsletter_with_list_unsubscribe_proposes_unsubscribe() {
        let mut f = base_facts(Category::Newsletter);
        f.has_list_unsubscribe = true;
        let actions = propose_actions(&f, &RuleConfig::default());
        assert!(actions.iter().any(|a| a.action == ActionType::Unsubscribe));
    }

    #[test]
    fn old_transactional_proposes_archive() {
        let mut f = base_facts(Category::Transactional);
        f.oldest_message_age_days = Some(400);
        let actions = propose_actions(&f, &RuleConfig::default());
        assert!(actions.iter().any(|a| a.action == ActionType::Archive));
    }
}
