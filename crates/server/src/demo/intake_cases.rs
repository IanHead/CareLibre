//! Sample intake cases for the demo seed.
//!
//! Cases are spread across every active flow and shared board column so the
//! intake board, case detail, timeline and waitlist screens have realistic data
//! to show. Prospect names come from the same [`NameBook`] as every other demo
//! contact, so the dataset keeps its "repeated names are rare" property.

use std::collections::HashMap;

use anyhow::{bail, Context};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use rand::rngs::StdRng;
use rand::Rng;
use serde_json::{json, Map, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::names::{Background, Cohort, Gender, NameBook};
use super::{phone, EMAIL_DOMAIN};

const ENQUIRY: &str = "enquiry";
const ELIGIBILITY: &str = "eligibility";
const RISK_ASSESSMENT: &str = "risk_assessment";
const SERVICE_PLANNING: &str = "service_planning";

struct Flow {
    id: Uuid,
    subject_kind: String,
    stages: HashMap<String, Stage>,
}

struct Stage {
    id: Uuid,
    name: String,
    kind: String,
    sla_hours: Option<i32>,
}

struct SeedEnv<'a> {
    organisation: Uuid,
    flows: &'a HashMap<String, Flow>,
    users: &'a HashMap<String, Uuid>,
    divisions: &'a [Uuid],
    services: &'a [Uuid],
    external: &'a HashMap<String, Uuid>,
    referrer: Option<Uuid>,
}

struct Prospect {
    id: Uuid,
    display_name: String,
    first_name: String,
    family_name: String,
    email: String,
    phone: String,
    date_of_birth: NaiveDate,
}

#[derive(Clone, Copy)]
struct CaseSpec {
    flow: &'static str,
    stage: &'static str,
    age: i32,
    priority: i16,
    division: usize,
    assignee: &'static str,
    source: &'static str,
    referral: Option<&'static str>,
    referred_by_contact: bool,
    external: Option<&'static str>,
    waitlist: Option<(usize, &'static str)>,
    days_in_stage: i64,
    notes: &'static str,
}

impl CaseSpec {
    fn new(flow: &'static str, stage: &'static str, age: i32) -> Self {
        Self {
            flow,
            stage,
            age,
            priority: 0,
            division: 0,
            assignee: "intake@bluedreams.example",
            source: "enquiry",
            referral: None,
            referred_by_contact: false,
            external: None,
            waitlist: None,
            days_in_stage: 3,
            notes: "",
        }
    }
}

/// Insert the sample cases and waitlist entries.
///
/// Returns the number of cases and waitlist entries created.
pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    rng: &mut StdRng,
    organisation: Uuid,
    names: &mut NameBook,
    divisions: &[Uuid],
    services: &[Uuid],
) -> anyhow::Result<(usize, usize)> {
    let flows = load_flows(tx, organisation).await?;
    let users = load_users(tx).await?;
    let external = load_external_organisations(tx, organisation).await?;
    let referrer = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM contacts
         WHERE organisation_id = $1 AND details->>'subtype' = 'external'
         ORDER BY display_name, id
         LIMIT 1",
    )
    .bind(organisation)
    .fetch_optional(&mut **tx)
    .await?;

    let env = SeedEnv {
        organisation,
        flows: &flows,
        users: &users,
        divisions,
        services,
        external: &external,
        referrer,
    };
    let specs = sample_cases();
    let mut waitlist_entries = 0;
    let now = Utc::now();

    for (index, spec) in specs.iter().enumerate() {
        if spec.waitlist.is_some() {
            waitlist_entries += 1;
        }
        insert_case(tx, rng, names, &env, spec, index, now).await?;
    }

    Ok((specs.len(), waitlist_entries))
}

async fn load_flows(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
) -> anyhow::Result<HashMap<String, Flow>> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            Uuid,
            String,
            String,
            Uuid,
            String,
            String,
            Option<i32>,
        ),
    >(
        "SELECT f.key, f.id, f.subject_kind, s.key, s.id, s.name, s.stage_kind, s.sla_hours
         FROM intake_flows f
         JOIN intake_stages s
           ON s.organisation_id = f.organisation_id AND s.flow_id = f.id
         WHERE f.organisation_id = $1 AND f.is_active
         ORDER BY f.sort_order, s.sort_order",
    )
    .bind(organisation)
    .fetch_all(&mut **tx)
    .await?;

    let mut flows: HashMap<String, Flow> = HashMap::new();
    for (flow_key, flow_id, subject_kind, stage_key, stage_id, stage_name, stage_kind, sla_hours) in
        rows
    {
        let flow = flows.entry(flow_key).or_insert_with(|| Flow {
            id: flow_id,
            subject_kind,
            stages: HashMap::new(),
        });
        flow.stages.insert(
            stage_key,
            Stage {
                id: stage_id,
                name: stage_name,
                kind: stage_kind,
                sla_hours,
            },
        );
    }
    Ok(flows)
}

async fn load_users(tx: &mut Transaction<'_, Postgres>) -> anyhow::Result<HashMap<String, Uuid>> {
    let rows = sqlx::query_as::<_, (String, Uuid)>(
        "SELECT lower(email), id FROM users WHERE lower(email) LIKE $1",
    )
    .bind(format!("%@{EMAIL_DOMAIN}"))
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().collect())
}

async fn load_external_organisations(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
) -> anyhow::Result<HashMap<String, Uuid>> {
    let rows = sqlx::query_as::<_, (String, Uuid)>(
        "SELECT name, id FROM organisations WHERE tenant_id = $1 AND id <> $1",
    )
    .bind(organisation)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().collect())
}

#[allow(clippy::too_many_arguments)]
async fn insert_case(
    tx: &mut Transaction<'_, Postgres>,
    rng: &mut StdRng,
    names: &mut NameBook,
    env: &SeedEnv<'_>,
    spec: &CaseSpec,
    index: usize,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let flow = env
        .flows
        .get(spec.flow)
        .with_context(|| format!("missing demo intake flow {}", spec.flow))?;
    let stage = flow
        .stages
        .get(spec.stage)
        .with_context(|| format!("missing stage {} for {}", spec.stage, spec.flow))?;

    let final_index = match spec.stage {
        ENQUIRY => 0,
        ELIGIBILITY => 1,
        RISK_ASSESSMENT => 2,
        SERVICE_PLANNING => 3,
        "onboarded" | "cancelled" => 4,
        other => bail!("unknown demo intake stage {other}"),
    };
    let terminal = if spec.stage == "onboarded" {
        "onboarded"
    } else {
        "cancelled"
    };
    let stage_keys = [
        ENQUIRY,
        ELIGIBILITY,
        RISK_ASSESSMENT,
        SERVICE_PLANNING,
        terminal,
    ];
    let path: Vec<&Stage> = stage_keys[..=final_index]
        .iter()
        .map(|key| {
            flow.stages
                .get(*key)
                .with_context(|| format!("missing stage {key} for {}", spec.flow))
        })
        .collect::<anyhow::Result<_>>()?;

    let jitter_hours = rng.gen_range(0..12);
    let final_entered =
        now - Duration::days(spec.days_in_stage) - Duration::hours(i64::from(jitter_hours));
    let entered: Vec<DateTime<Utc>> = (0..=final_index)
        .map(|position| final_entered - Duration::days(((final_index - position) * 2) as i64))
        .collect();
    let created_at = entered[0];
    let stage_entered_at = *entered.last().expect("at least one stage");

    let lifecycle = match stage.kind.as_str() {
        "enquiry" => "enquiry",
        "onboarded" => "active",
        "cancelled" => "intake_cancelled",
        _ => "onboarding",
    };
    let contact = if flow.subject_kind == "organisation" {
        None
    } else {
        Some(insert_prospect(tx, rng, env.organisation, names, index, spec.age, lifecycle).await?)
    };
    let external_id = match spec.external {
        Some(name) => Some(
            *env.external
                .get(name)
                .with_context(|| format!("missing demo organisation {name}"))?,
        ),
        None => None,
    };
    let display_name = match (&contact, spec.external) {
        (Some(contact), _) => contact.display_name.clone(),
        (None, Some(name)) => name.to_owned(),
        (None, None) => bail!("intake case needs a contact or organisation"),
    };

    let assignee = env
        .users
        .get(spec.assignee)
        .or_else(|| env.users.get("intake@bluedreams.example"))
        .copied()
        .context("missing demo intake assignee")?;
    let created_by = env
        .users
        .get("owner@bluedreams.example")
        .copied()
        .unwrap_or(assignee);
    let division = env
        .divisions
        .get(spec.division % env.divisions.len())
        .copied();

    let (referrer_contact_id, referrer_organisation_id) = if spec.source == "referral" {
        if spec.referred_by_contact && env.referrer.is_some() {
            (env.referrer, None)
        } else {
            (
                None,
                spec.referral
                    .and_then(|name| env.external.get(name).copied()),
            )
        }
    } else {
        (None, None)
    };
    let referral_reference =
        (spec.source == "referral").then(|| format!("REF-{}-{:04}", now.year(), index + 1));
    let referral_received_at = (spec.source == "referral").then_some(created_at);
    let onboarded_at = (stage.kind == "onboarded").then_some(stage_entered_at);
    let closed_at = (stage.kind == "cancelled").then_some(stage_entered_at);
    let cancellation_reason = (stage.kind == "cancelled").then(|| {
        if spec.notes.is_empty() {
            "Client withdrew from intake".to_owned()
        } else {
            spec.notes.to_owned()
        }
    });
    let stage_due_at = stage
        .sla_hours
        .map(|hours| stage_entered_at + Duration::hours(i64::from(hours)));
    let data = case_data(spec, &contact, flow, final_index);

    let case_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO intake_cases
            (id, organisation_id, flow_id, stage_id, contact_id, external_organisation_id,
             subject_kind, display_name, data, source, referrer_contact_id,
             referrer_organisation_id, referral_reference, referral_received_at,
             division_id, assignee_user_id, priority, stage_entered_at, stage_due_at,
             version, created_by, created_at, updated_at, onboarded_at, closed_at,
             cancellation_reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                 $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)",
    )
    .bind(case_id)
    .bind(env.organisation)
    .bind(flow.id)
    .bind(stage.id)
    .bind(contact.as_ref().map(|contact| contact.id))
    .bind(external_id)
    .bind(&flow.subject_kind)
    .bind(&display_name)
    .bind(&data)
    .bind(spec.source)
    .bind(referrer_contact_id)
    .bind(referrer_organisation_id)
    .bind(&referral_reference)
    .bind(referral_received_at)
    .bind(division)
    .bind(Some(assignee))
    .bind(spec.priority)
    .bind(stage_entered_at)
    .bind(stage_due_at)
    .bind(1_i32)
    .bind(created_by)
    .bind(created_at)
    .bind(stage_entered_at)
    .bind(onboarded_at)
    .bind(closed_at)
    .bind(&cancellation_reason)
    .execute(&mut **tx)
    .await?;

    if let Some(contact) = &contact {
        insert_status_history(
            tx,
            env.organisation,
            contact.id,
            None,
            "enquiry",
            "Intake enquiry created",
            created_by,
            created_at,
        )
        .await?;
        if final_index >= 1 {
            insert_status_history(
                tx,
                env.organisation,
                contact.id,
                Some("enquiry"),
                "onboarding",
                &format!("Intake stage: {}", path[1].name),
                assignee,
                entered[1],
            )
            .await?;
        }
        if stage.kind == "onboarded" {
            insert_status_history(
                tx,
                env.organisation,
                contact.id,
                Some("onboarding"),
                "active",
                "Intake stage: Onboarded",
                assignee,
                entered[4],
            )
            .await?;
        }
        if stage.kind == "cancelled" {
            insert_status_history(
                tx,
                env.organisation,
                contact.id,
                Some("onboarding"),
                "intake_cancelled",
                cancellation_reason.as_deref().unwrap_or("Intake cancelled"),
                assignee,
                entered[4],
            )
            .await?;
        }
    }

    crate::intake::cases::materialise_checklist(tx, env.organisation, case_id, stage.id).await?;
    let checklist_done = match stage.kind.as_str() {
        "enquiry" => 1,
        "eligibility" => 2,
        "risk_assessment" => 2,
        "service_planning" => {
            if spec.waitlist.is_some() {
                2
            } else {
                1
            }
        }
        "onboarded" => 2,
        "cancelled" => 1,
        _ => 0,
    };
    sqlx::query(
        "UPDATE intake_case_checklist_items
         SET completed_at = $1, completed_by = $2
         WHERE id IN (
             SELECT id FROM intake_case_checklist_items
             WHERE organisation_id = $3 AND case_id = $4 AND stage_id = $5
             ORDER BY sort_order, id
             LIMIT $6
         )",
    )
    .bind(stage_entered_at + Duration::hours(2))
    .bind(assignee)
    .bind(env.organisation)
    .bind(case_id)
    .bind(stage.id)
    .bind(checklist_done)
    .execute(&mut **tx)
    .await?;

    insert_event(
        tx,
        env.organisation,
        case_id,
        "created",
        created_by,
        None,
        Some(path[0].id),
        json!({}),
        created_at,
    )
    .await?;
    insert_event(
        tx,
        env.organisation,
        case_id,
        "assigned",
        created_by,
        None,
        None,
        json!({ "assignee_user_id": assignee }),
        created_at + Duration::hours(1),
    )
    .await?;
    for position in 1..=final_index {
        insert_event(
            tx,
            env.organisation,
            case_id,
            "transitioned",
            assignee,
            Some(path[position - 1].id),
            Some(path[position].id),
            json!({}),
            entered[position],
        )
        .await?;
    }
    if stage.kind == "onboarded" {
        insert_event(
            tx,
            env.organisation,
            case_id,
            "onboarded",
            assignee,
            None,
            Some(stage.id),
            json!({}),
            stage_entered_at + Duration::hours(1),
        )
        .await?;
    }
    if stage.kind == "cancelled" {
        insert_event(
            tx,
            env.organisation,
            case_id,
            "cancelled",
            assignee,
            None,
            Some(stage.id),
            json!({ "reason": cancellation_reason }),
            stage_entered_at + Duration::hours(1),
        )
        .await?;
    }

    if let Some((service_index, status)) = spec.waitlist {
        let service_id = env.services.get(service_index).copied();
        let requested_start = now.date_naive() + Duration::days(14);
        let (offered_at, offer_expires_at) = if status == "offered" {
            (
                Some(stage_entered_at),
                Some(stage_entered_at + Duration::days(3)),
            )
        } else {
            (None, None)
        };
        sqlx::query(
            "INSERT INTO intake_waitlist_entries
                (id, organisation_id, case_id, service_id, division_id, priority,
                 requested_start, status, offered_at, offer_expires_at, notes,
                 created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(Uuid::now_v7())
        .bind(env.organisation)
        .bind(case_id)
        .bind(service_id)
        .bind(division)
        .bind(spec.priority)
        .bind(requested_start)
        .bind(status)
        .bind(offered_at)
        .bind(offer_expires_at)
        .bind(waitlist_note(status))
        .bind(stage_entered_at)
        .bind(stage_entered_at)
        .execute(&mut **tx)
        .await?;
        insert_event(
            tx,
            env.organisation,
            case_id,
            "waitlisted",
            assignee,
            None,
            None,
            json!({ "service_id": service_id, "status": status }),
            stage_entered_at,
        )
        .await?;
    }

    if stage.kind == "onboarded" {
        if let Some(contact) = &contact {
            crate::intake::cases::link_role(tx, env.organisation, contact.id, &flow.subject_kind)
                .await?;
        }
    }

    Ok(())
}

async fn insert_prospect(
    tx: &mut Transaction<'_, Postgres>,
    rng: &mut StdRng,
    organisation: Uuid,
    names: &mut NameBook,
    index: usize,
    age: i32,
    lifecycle: &str,
) -> anyhow::Result<Prospect> {
    let background = Background::random(rng, Cohort::General);
    let gender = Gender::random(rng, 0.5);
    let birth_year = Utc::now().date_naive().year() - age;
    let name = names.name(rng, gender, birth_year, background, None);
    let day = rng.gen_range(1..=365);
    let date_of_birth = NaiveDate::from_yo_opt(birth_year, day)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(birth_year, 6, 30).expect("valid date"));
    let display_name = name.display();
    let email = format!("intake.{index:02}@{EMAIL_DOMAIN}");
    let phone_number = phone(rng);
    let id = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO contacts
            (id, organisation_id, display_name, given_name, family_name, preferred_name,
             pronouns, date_of_birth, email, phone, notes, details, lifecycle_status)
         VALUES ($1, $2, $3, $4, $5, NULL, NULL, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(organisation)
    .bind(&display_name)
    .bind(&name.given)
    .bind(&name.family)
    .bind(date_of_birth)
    .bind(&email)
    .bind(&phone_number)
    .bind("Intake enquiry created by the demo seed.")
    .bind(json!({ "subtype": "intake" }))
    .bind(lifecycle)
    .execute(&mut **tx)
    .await?;

    Ok(Prospect {
        id,
        display_name,
        first_name: name.given,
        family_name: name.family,
        email,
        phone: phone_number,
        date_of_birth,
    })
}

fn case_data(
    spec: &CaseSpec,
    contact: &Option<Prospect>,
    flow: &Flow,
    final_index: usize,
) -> Value {
    let mut data = Map::new();
    if let Some(contact) = contact {
        data.insert("first_name".to_owned(), json!(&contact.first_name));
        data.insert("family_name".to_owned(), json!(&contact.family_name));
        data.insert("email".to_owned(), json!(&contact.email));
        data.insert("phone".to_owned(), json!(&contact.phone));
        data.insert(
            "date_of_birth".to_owned(),
            json!(contact.date_of_birth.to_string()),
        );
    } else if let Some(name) = spec.external {
        data.insert("organisation_name".to_owned(), json!(name));
        data.insert(
            "email".to_owned(),
            json!(format!(
                "intake.{}@{}",
                name.to_lowercase().replace(' ', "."),
                EMAIL_DOMAIN
            )),
        );
        data.insert("phone".to_owned(), json!("03 9000 0000"));
    }

    if final_index >= 1 {
        if flow.subject_kind == "client" {
            data.insert("funding".to_owned(), json!(funding_for(spec.flow)));
        }
        data.insert("consent_services".to_owned(), json!(true));
        data.insert("consent_share".to_owned(), json!(true));
    }
    if final_index >= 2 {
        data.insert(
            "risk_summary".to_owned(),
            json!(risk_summary(spec.priority)),
        );
        data.insert(
            "behaviour_notes".to_owned(),
            json!("No behaviours of concern reported. Usual routines and preferences apply."),
        );
    }
    if final_index >= 3 {
        data.insert("support_needs".to_owned(), json!(support_needs(spec.flow)));
        data.insert(
            "schedule_preferences".to_owned(),
            json!("Weekday mornings, preferably with the same worker each week."),
        );
    }
    if flow.subject_kind == "worker" && final_index >= 1 {
        data.insert("position".to_owned(), json!("Support worker"));
        data.insert("employment_type".to_owned(), json!("casual"));
        data.insert(
            "award".to_owned(),
            json!("Social, Community, Home Care and Disability Services Industry Award"),
        );
        data.insert("classification".to_owned(), json!("Level 2.3"));
        data.insert("pay_point".to_owned(), json!("North"));
        data.insert("superannuation".to_owned(), json!("Aware Super"));
        data.insert(
            "start_date".to_owned(),
            json!((Utc::now().date_naive() - Duration::days(10)).to_string()),
        );
    }
    if !spec.notes.is_empty() {
        data.insert("notes".to_owned(), json!(spec.notes));
    }
    Value::Object(data)
}

fn funding_for(flow: &str) -> &'static str {
    match flow {
        "client_aged_care" => "aged_care",
        "client_fee_for_service" => "private",
        _ => "ndis",
    }
}

fn support_needs(flow: &str) -> &'static str {
    match flow {
        "client_aged_care" => {
            "Assistance with showering, dressing, meal preparation and fortnightly cleaning."
        }
        "client_fee_for_service" => "Weekly social support and transport to appointments.",
        "worker_standard" => "Available for personal care, community access and respite shifts.",
        _ => "Assistance with personal care, meal preparation and community access.",
    }
}

fn risk_summary(priority: i16) -> &'static str {
    match priority {
        2 => "High: recent safeguarding concern. Two workers required for community access.",
        1 => "Falls risk and occasional confusion. Supervision on stairs and in the bathroom.",
        _ => "No known risks. Standard infection control and manual handling apply.",
    }
}

fn waitlist_note(status: &str) -> &'static str {
    match status {
        "offered" => "Place offered; family is confirming the start date.",
        "placed" => "Place confirmed for the next available intake.",
        _ => "Waiting for a vacancy in the north.",
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_status_history(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    contact_id: Uuid,
    from: Option<&str>,
    to: &str,
    reason: &str,
    actor: Uuid,
    created_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO contact_status_history
            (id, organisation_id, contact_id, from_status, to_status, reason,
             actor_user_id, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(contact_id)
    .bind(from)
    .bind(to)
    .bind(reason)
    .bind(actor)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_event(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    case_id: Uuid,
    event_type: &str,
    actor: Uuid,
    from_stage: Option<Uuid>,
    to_stage: Option<Uuid>,
    metadata: Value,
    created_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO intake_case_events
            (organisation_id, case_id, event_type, actor_user_id, from_stage_id,
             to_stage_id, metadata, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(organisation)
    .bind(case_id)
    .bind(event_type)
    .bind(actor)
    .bind(from_stage)
    .bind(to_stage)
    .bind(metadata)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::vec_init_then_push)]
fn sample_cases() -> Vec<CaseSpec> {
    let mut cases = Vec::new();

    // Disability standard: the fullest pipeline, including a cancellation.
    cases.push(CaseSpec {
        priority: 1,
        source: "referral",
        referral: Some("Northside Medical Centre"),
        days_in_stage: 2,
        notes: "GP referral for community access and personal care.",
        ..CaseSpec::new("client_disability_standard", ENQUIRY, 27)
    });
    cases.push(CaseSpec {
        source: "internal",
        days_in_stage: 1,
        ..CaseSpec::new("client_disability_standard", ENQUIRY, 14)
    });
    cases.push(CaseSpec {
        days_in_stage: 3,
        ..CaseSpec::new("client_disability_standard", ELIGIBILITY, 34)
    });
    cases.push(CaseSpec {
        priority: 1,
        source: "referral",
        referral: Some("Care Connect Support Coordination"),
        assignee: "admin@bluedreams.example",
        days_in_stage: 4,
        ..CaseSpec::new("client_disability_standard", ELIGIBILITY, 52)
    });
    cases.push(CaseSpec {
        source: "internal",
        days_in_stage: 5,
        ..CaseSpec::new("client_disability_standard", RISK_ASSESSMENT, 23)
    });
    cases.push(CaseSpec {
        priority: 2,
        source: "referral",
        referral: Some("Advocacy Victoria"),
        referred_by_contact: true,
        assignee: "owner@bluedreams.example",
        days_in_stage: 2,
        notes: "Urgent: safeguarding concern raised by family.",
        ..CaseSpec::new("client_disability_standard", RISK_ASSESSMENT, 41)
    });
    cases.push(CaseSpec {
        waitlist: Some((2, "waiting")),
        days_in_stage: 6,
        notes: "Family requested respite during the school holidays.",
        ..CaseSpec::new("client_disability_standard", SERVICE_PLANNING, 19)
    });
    cases.push(CaseSpec {
        days_in_stage: 1,
        ..CaseSpec::new("client_disability_standard", "onboarded", 30)
    });
    cases.push(CaseSpec {
        days_in_stage: 3,
        notes: "Client moved interstate.",
        ..CaseSpec::new("client_disability_standard", "cancelled", 45)
    });

    // Disability emergency: short, high-priority stays.
    cases.push(CaseSpec {
        priority: 2,
        source: "referral",
        referral: Some("Advocacy Victoria"),
        assignee: "owner@bluedreams.example",
        days_in_stage: 0,
        notes: "Emergency respite requested after hospital discharge.",
        ..CaseSpec::new("client_disability_emergency", ENQUIRY, 17)
    });
    cases.push(CaseSpec {
        priority: 1,
        source: "internal",
        days_in_stage: 2,
        ..CaseSpec::new("client_disability_emergency", ELIGIBILITY, 9)
    });
    cases.push(CaseSpec {
        priority: 1,
        waitlist: Some((3, "offered")),
        assignee: "admin@bluedreams.example",
        days_in_stage: 3,
        ..CaseSpec::new("client_disability_emergency", SERVICE_PLANNING, 36)
    });

    // Aged care: referrals, a risk case and a short wait.
    cases.push(CaseSpec {
        priority: 1,
        source: "referral",
        referral: Some("Northside Medical Centre"),
        days_in_stage: 1,
        ..CaseSpec::new("client_aged_care", ENQUIRY, 82)
    });
    cases.push(CaseSpec {
        source: "internal",
        assignee: "admin@bluedreams.example",
        days_in_stage: 4,
        ..CaseSpec::new("client_aged_care", ELIGIBILITY, 78)
    });
    cases.push(CaseSpec {
        priority: 1,
        source: "referral",
        referral: Some("Melbourne Allied Health"),
        referred_by_contact: true,
        days_in_stage: 2,
        notes: "Falls risk and cognitive changes noted by family.",
        ..CaseSpec::new("client_aged_care", RISK_ASSESSMENT, 86)
    });
    cases.push(CaseSpec {
        waitlist: Some((5, "waiting")),
        days_in_stage: 5,
        ..CaseSpec::new("client_aged_care", SERVICE_PLANNING, 74)
    });
    cases.push(CaseSpec {
        days_in_stage: 2,
        ..CaseSpec::new("client_aged_care", "onboarded", 80)
    });

    // Fee for service: private enquiries.
    cases.push(CaseSpec {
        days_in_stage: 1,
        ..CaseSpec::new("client_fee_for_service", ENQUIRY, 39)
    });
    cases.push(CaseSpec {
        source: "referral",
        referral: Some("Melbourne Allied Health"),
        days_in_stage: 2,
        ..CaseSpec::new("client_fee_for_service", ELIGIBILITY, 61)
    });

    // Workers.
    cases.push(CaseSpec {
        source: "internal",
        days_in_stage: 1,
        notes: "Experienced support worker, available weekday mornings.",
        ..CaseSpec::new("worker_standard", ENQUIRY, 24)
    });
    cases.push(CaseSpec {
        source: "referral",
        referral: Some("Community Legal Help"),
        referred_by_contact: true,
        assignee: "admin@bluedreams.example",
        days_in_stage: 3,
        ..CaseSpec::new("worker_standard", ELIGIBILITY, 31)
    });
    cases.push(CaseSpec {
        days_in_stage: 2,
        ..CaseSpec::new("worker_standard", "onboarded", 43)
    });

    // Other contacts: a GP liaison and a coordinator.
    cases.push(CaseSpec {
        source: "referral",
        referral: Some("Northside Medical Centre"),
        days_in_stage: 2,
        notes: "New GP liaison contact at the practice.",
        ..CaseSpec::new("other_contact_standard", ENQUIRY, 58)
    });
    cases.push(CaseSpec {
        source: "internal",
        days_in_stage: 4,
        ..CaseSpec::new("other_contact_standard", ELIGIBILITY, 66)
    });

    // Organisations: a transport provider and an equipment supplier.
    cases.push(CaseSpec {
        external: Some("Accessible Transit"),
        source: "referral",
        referral: Some("Care Connect Support Coordination"),
        days_in_stage: 1,
        notes: "New transport provider for the north.",
        ..CaseSpec::new("organisation_standard", ENQUIRY, 0)
    });
    cases.push(CaseSpec {
        external: Some("Independent Living Supplies"),
        priority: 1,
        source: "internal",
        assignee: "admin@bluedreams.example",
        days_in_stage: 3,
        ..CaseSpec::new("organisation_standard", ELIGIBILITY, 0)
    });

    cases
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn samples_cover_every_active_flow_and_stage() {
        let cases = sample_cases();
        let flows: HashSet<_> = cases.iter().map(|case| case.flow).collect();
        for flow in [
            "client_aged_care",
            "client_disability_standard",
            "client_disability_emergency",
            "client_fee_for_service",
            "worker_standard",
            "other_contact_standard",
            "organisation_standard",
        ] {
            assert!(flows.contains(flow), "missing sample case for {flow}");
        }

        let stages: HashSet<_> = cases.iter().map(|case| case.stage).collect();
        for stage in [
            ENQUIRY,
            ELIGIBILITY,
            RISK_ASSESSMENT,
            SERVICE_PLANNING,
            "onboarded",
            "cancelled",
        ] {
            assert!(stages.contains(stage), "missing sample case in {stage}");
        }
    }

    #[test]
    fn organisation_cases_name_an_external_organisation() {
        for case in sample_cases()
            .iter()
            .filter(|case| case.flow == "organisation_standard")
        {
            assert!(case.external.is_some());
            assert!(!case.referred_by_contact);
        }
    }

    #[test]
    fn referral_cases_name_a_referrer() {
        for case in sample_cases()
            .iter()
            .filter(|case| case.source == "referral")
        {
            assert!(case.referral.is_some());
        }
    }
}
