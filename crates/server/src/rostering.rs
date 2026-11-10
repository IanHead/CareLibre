//! Rostering suitability scoring.
//!
//! Evidence comes from rostering history views and explicit constraints. The
//! scoring function is deliberately transparent: every contribution becomes a
//! reason or warning that a coordinator can inspect.

use chrono::{DateTime, Utc};

/// Aggregated observed history for one relationship or context.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistorySignal {
    pub attended_shifts: i64,
    pub no_show_shifts: i64,
    pub cancelled_shifts: i64,
    pub last_worked_at: Option<DateTime<Utc>>,
    pub average_rating: Option<f64>,
    pub positive_outcomes: i64,
    pub negative_outcomes: i64,
}

impl HistorySignal {
    #[must_use]
    pub fn has_evidence(&self) -> bool {
        self.attended_shifts > 0
            || self.no_show_shifts > 0
            || self.cancelled_shifts > 0
            || self.average_rating.is_some()
            || self.positive_outcomes > 0
            || self.negative_outcomes > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    Preferred,
    Avoid,
    Blocked,
}

impl ConstraintKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preferred => "preferred",
            Self::Avoid => "avoid",
            Self::Blocked => "blocked",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "preferred" => Some(Self::Preferred),
            "avoid" => Some(Self::Avoid),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConstraintEvidence {
    pub kind: ConstraintKind,
    pub strength: i16,
    pub target_label: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContextEvidence {
    pub label: String,
    pub signal: HistorySignal,
    pub multiplier: f64,
}

/// How strongly a client requirement applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementSeverity {
    Warn,
    Block,
}

impl RequirementSeverity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warn => "warn",
            Self::Block => "block",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "warn" => Some(Self::Warn),
            "block" => Some(Self::Block),
            _ => None,
        }
    }
}

/// The result of matching one client requirement against one worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementStatus {
    Met,
    Expiring,
    Expired,
    Mismatch,
    Unknown,
}

impl RequirementStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Met => "met",
            Self::Expiring => "expiring",
            Self::Expired => "expired",
            Self::Mismatch => "mismatch",
            Self::Unknown => "unknown",
        }
    }

    /// `Expiring` still meets the requirement; it is only a warning.
    #[must_use]
    pub const fn is_met(self) -> bool {
        matches!(self, Self::Met | Self::Expiring)
    }
}

/// One client requirement evaluated against a candidate.
#[derive(Debug, Clone)]
pub struct RequirementEvidence {
    pub quality_name: String,
    pub client_name: String,
    pub status: RequirementStatus,
    pub severity: RequirementSeverity,
    /// Short explanation such as "expires 2027-01-10" or "recorded as male".
    pub detail: Option<String>,
}

pub struct ScoreInput<'a> {
    pub client_history: &'a [(String, HistorySignal)],
    pub coworker_history: &'a [(String, HistorySignal)],
    pub context_history: &'a [ContextEvidence],
    pub constraints: &'a [ConstraintEvidence],
    pub requirements: &'a [RequirementEvidence],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoreResult {
    pub score: f64,
    pub blocked: bool,
    /// False when at least one client requirement is not met. Such a candidate
    /// is still assignable; the desktop groups them after those that meet all
    /// requirements.
    pub meets_requirements: bool,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

/// Score a candidate on a 0–100 scale from observed evidence and constraints.
#[must_use]
pub fn score_candidate(input: ScoreInput<'_>) -> ScoreResult {
    let mut score = 50.0;
    let mut reasons = Vec::new();
    let mut warnings = Vec::new();

    for (label, signal) in input.client_history {
        add_client_evidence(&mut score, label, signal, &mut reasons, &mut warnings);
    }

    for (label, signal) in input.coworker_history {
        add_history(
            &mut score,
            signal,
            2.0,
            &format!("attended shift with {label}"),
            &mut reasons,
            &mut warnings,
        );
    }

    for context in input.context_history {
        add_context_evidence(&mut score, context, &mut reasons, &mut warnings);
    }

    let mut blocked = false;
    let mut meets_requirements = true;
    for requirement in input.requirements {
        if !requirement.status.is_met() {
            meets_requirements = false;
            if requirement.severity == RequirementSeverity::Block {
                blocked = true;
            }
        }

        let detail = requirement
            .detail
            .as_deref()
            .filter(|detail| !detail.is_empty())
            .map(|detail| format!(" ({detail})"))
            .unwrap_or_default();

        match requirement.status {
            RequirementStatus::Met => reasons.push(format!(
                "meets {}'s {} requirement{detail}",
                requirement.client_name, requirement.quality_name
            )),
            RequirementStatus::Expiring => warnings.push(format!(
                "{} for {} expires soon{detail}",
                requirement.quality_name, requirement.client_name
            )),
            RequirementStatus::Expired => {
                score -= 8.0;
                warnings.push(format!(
                    "{} for {} has expired{detail}",
                    requirement.quality_name, requirement.client_name
                ));
            }
            RequirementStatus::Mismatch => {
                score -= 8.0;
                warnings.push(format!(
                    "{} does not meet {}'s requirement{detail}",
                    requirement.quality_name, requirement.client_name
                ));
            }
            RequirementStatus::Unknown => {
                score -= 8.0;
                warnings.push(format!(
                    "{} not recorded; required by {}{detail}",
                    requirement.quality_name, requirement.client_name
                ));
            }
        }

        if !requirement.status.is_met() && requirement.severity == RequirementSeverity::Block {
            warnings.push(format!(
                "blocked: {} is required by {}",
                requirement.quality_name, requirement.client_name
            ));
        }
    }

    for constraint in input.constraints {
        let strength = f64::from(constraint.strength.clamp(1, 5));
        let detail = constraint
            .reason
            .as_deref()
            .filter(|reason| !reason.is_empty())
            .map(|reason| format!(": {reason}"))
            .unwrap_or_default();

        match constraint.kind {
            ConstraintKind::Preferred => {
                score += 8.0 * strength;
                reasons.push(format!("preferred for {}{detail}", constraint.target_label));
            }
            ConstraintKind::Avoid => {
                score -= 8.0 * strength;
                warnings.push(format!("avoid {}{detail}", constraint.target_label));
            }
            ConstraintKind::Blocked => {
                blocked = true;
                warnings.push(format!("blocked for {}{detail}", constraint.target_label));
            }
        }
    }

    ScoreResult {
        score: round1(score.clamp(0.0, 100.0)),
        blocked,
        meets_requirements,
        reasons,
        warnings,
    }
}

fn add_client_evidence(
    score: &mut f64,
    label: &str,
    signal: &HistorySignal,
    reasons: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let recency = recency_factor(signal.last_worked_at);
    if signal.attended_shifts > 0 {
        *score += signal.attended_shifts as f64 * 4.0 * recency;
        reasons.push(format!(
            "{} attended shift{} with {label}",
            signal.attended_shifts,
            plural(signal.attended_shifts)
        ));
    }
    if let Some(rating) = signal.average_rating {
        *score += (rating - 3.0) * 3.0;
        reasons.push(format!("average rating {rating:.1}/5 with {label}"));
    }
    if signal.positive_outcomes > 0 {
        *score += signal.positive_outcomes as f64 * 3.0;
        reasons.push(format!(
            "{} positive outcome{} with {label}",
            signal.positive_outcomes,
            plural(signal.positive_outcomes)
        ));
    }
    if signal.negative_outcomes > 0 {
        *score -= signal.negative_outcomes as f64 * 6.0;
        warnings.push(format!(
            "{} negative outcome{} with {label}",
            signal.negative_outcomes,
            plural(signal.negative_outcomes)
        ));
    }
    if signal.no_show_shifts > 0 {
        *score -= signal.no_show_shifts as f64 * 3.0;
        warnings.push(format!(
            "{} no-show{} with {label}",
            signal.no_show_shifts,
            plural(signal.no_show_shifts)
        ));
    }
    if signal.cancelled_shifts > 0 {
        *score -= signal.cancelled_shifts as f64 * 0.5;
    }
}

fn add_context_evidence(
    score: &mut f64,
    context: &ContextEvidence,
    reasons: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let recency = recency_factor(context.signal.last_worked_at);
    if context.signal.attended_shifts > 0 {
        *score += context.signal.attended_shifts as f64 * context.multiplier * recency;
        reasons.push(format!(
            "{} attended shift{} at {}",
            context.signal.attended_shifts,
            plural(context.signal.attended_shifts),
            context.label
        ));
    }
    if context.signal.no_show_shifts > 0 {
        *score -= context.signal.no_show_shifts as f64 * context.multiplier * 0.5;
        warnings.push(format!(
            "{} no-show{} at {}",
            context.signal.no_show_shifts,
            plural(context.signal.no_show_shifts),
            context.label
        ));
    }
}

fn add_history(
    score: &mut f64,
    signal: &HistorySignal,
    multiplier: f64,
    label: &str,
    reasons: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let recency = recency_factor(signal.last_worked_at);
    if signal.attended_shifts > 0 {
        *score += signal.attended_shifts as f64 * multiplier * recency;
        reasons.push(format!("{} {}", signal.attended_shifts, label));
    }
    if signal.no_show_shifts > 0 {
        *score -= signal.no_show_shifts as f64 * multiplier * 0.5;
        warnings.push(format!(
            "{} no-show{} while {}",
            signal.no_show_shifts,
            plural(signal.no_show_shifts),
            label
        ));
    }
}

fn recency_factor(last_worked_at: Option<DateTime<Utc>>) -> f64 {
    let Some(last) = last_worked_at else {
        return 1.0;
    };
    let days = (Utc::now() - last).num_days().max(0) as f64;
    1.0 / (1.0 + days / 180.0)
}

const fn plural(count: i64) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(attended: i64) -> HistorySignal {
        HistorySignal {
            attended_shifts: attended,
            ..HistorySignal::default()
        }
    }

    #[test]
    fn no_evidence_is_neutral() {
        let result = score_candidate(ScoreInput {
            client_history: &[],
            coworker_history: &[],
            context_history: &[],
            constraints: &[],
            requirements: &[],
        });
        assert_eq!(result.score, 50.0);
        assert!(!result.blocked);
        assert!(result.reasons.is_empty());
    }

    #[test]
    fn client_history_raises_the_score() {
        let history = vec![("Jane Doe".to_owned(), signal(5))];
        let result = score_candidate(ScoreInput {
            client_history: &history,
            coworker_history: &[],
            context_history: &[],
            constraints: &[],
            requirements: &[],
        });
        assert!(result.score > 50.0);
        assert!(result
            .reasons
            .iter()
            .any(|reason| reason.contains("Jane Doe")));
    }

    #[test]
    fn negative_outcomes_lower_the_score() {
        let history = vec![(
            "Jane Doe".to_owned(),
            HistorySignal {
                attended_shifts: 2,
                negative_outcomes: 2,
                ..HistorySignal::default()
            },
        )];
        let result = score_candidate(ScoreInput {
            client_history: &history,
            coworker_history: &[],
            context_history: &[],
            constraints: &[],
            requirements: &[],
        });
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("negative")));
    }

    #[test]
    fn blocked_constraints_mark_the_candidate_ineligible() {
        let constraints = vec![ConstraintEvidence {
            kind: ConstraintKind::Blocked,
            strength: 5,
            target_label: "Alex Smith".to_owned(),
            reason: Some("do not roster together".to_owned()),
        }];
        let result = score_candidate(ScoreInput {
            client_history: &[],
            coworker_history: &[],
            context_history: &[],
            constraints: &constraints,
            requirements: &[],
        });
        assert!(result.blocked);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("blocked")));
    }

    #[test]
    fn constraints_are_ordered_by_kind() {
        assert_eq!(
            ConstraintKind::parse("preferred"),
            Some(ConstraintKind::Preferred)
        );
        assert_eq!(ConstraintKind::parse("AVOID"), Some(ConstraintKind::Avoid));
        assert_eq!(
            ConstraintKind::parse("blocked"),
            Some(ConstraintKind::Blocked)
        );
        assert_eq!(ConstraintKind::parse("unknown"), None);
    }

    #[test]
    fn unmet_requirements_warn_and_penalise_without_blocking() {
        let requirements = vec![RequirementEvidence {
            quality_name: "PEG feeding".to_owned(),
            client_name: "Margaret Chen".to_owned(),
            status: RequirementStatus::Unknown,
            severity: RequirementSeverity::Warn,
            detail: None,
        }];
        let result = score_candidate(ScoreInput {
            client_history: &[],
            coworker_history: &[],
            context_history: &[],
            constraints: &[],
            requirements: &requirements,
        });
        assert!(!result.blocked);
        assert!(!result.meets_requirements);
        assert!(result.score < 50.0);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("PEG feeding") && warning.contains("Margaret Chen")));
    }

    #[test]
    fn block_severity_is_opt_in() {
        let requirements = vec![RequirementEvidence {
            quality_name: "PEG feeding".to_owned(),
            client_name: "Margaret Chen".to_owned(),
            status: RequirementStatus::Expired,
            severity: RequirementSeverity::Block,
            detail: Some("expired 2026-01-01".to_owned()),
        }];
        let result = score_candidate(ScoreInput {
            client_history: &[],
            coworker_history: &[],
            context_history: &[],
            constraints: &[],
            requirements: &requirements,
        });
        assert!(result.blocked);
        assert!(!result.meets_requirements);
    }

    #[test]
    fn expiring_requirements_do_not_penalise() {
        let requirements = vec![RequirementEvidence {
            quality_name: "First aid".to_owned(),
            client_name: "Margaret Chen".to_owned(),
            status: RequirementStatus::Expiring,
            severity: RequirementSeverity::Warn,
            detail: Some("expires 2026-11-20".to_owned()),
        }];
        let result = score_candidate(ScoreInput {
            client_history: &[],
            coworker_history: &[],
            context_history: &[],
            constraints: &[],
            requirements: &requirements,
        });
        assert!(result.meets_requirements);
        assert_eq!(result.score, 50.0);
        assert!(!result.warnings.is_empty());
    }
}
