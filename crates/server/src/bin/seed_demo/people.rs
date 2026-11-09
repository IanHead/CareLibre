//! People, households and relationships for the demo seed.
//!
//! The aim is a support network that looks like a real disability and aged-care
//! provider's client list:
//!
//! - clients live in households: ageing couples who both receive aged care,
//!   brothers and sisters who are both NDIS participants, an ageing parent who
//!   still cares for an adult child with disability, and shared supported
//!   independent living (SIL) houses;
//! - family networks follow from age: young participants have parents (one of
//!   them the primary carer), adults may have a guardian, a sibling or a
//!   partner, and older clients have adult children, one of whom often holds an
//!   enduring power of attorney;
//! - each client is a patient of a named GP at a practice near home, with a
//!   specialist where it makes sense (paediatricians for children,
//!   geriatricians for the very old);
//! - a handful of workers are related to each other (couples, siblings, a
//!   mother and daughter), and a few are related to clients, which is declared
//!   as a conflict of interest.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Duration, NaiveDate, Utc};
use rand::{rngs::StdRng, Rng};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::names::{nickname, Background, Cohort, Gender, Name, NameBook};
use super::{abn, phone, set_custom_value, EMAIL_DOMAIN, EXTERNAL_ORGS, STREETS, SUBURBS};

const NDIS_CLIENTS: usize = 400;
const AGED_CLIENTS: usize = 200;
const STAFF: usize = 300;

const AGED_COUPLES: usize = 14;
const AGEING_PARENT_CARERS: usize = 6;
const SIBLING_GROUPS: &[usize] = &[2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3];
const SIL_HOUSES: &[usize] = &[3, 4, 3, 4, 3, 4];
const COUSIN_PAIRS: usize = 3;

const WORKER_COUPLES: usize = 6;
const WORKER_SIBLINGS: usize = 5;
const WORKER_PARENT_CHILD: usize = 4;
const WORKER_COUSINS: usize = 2;

const SPECIALISTS: &[(&str, usize)] = &[
    ("paediatrician", 2),
    ("geriatrician", 2),
    ("psychiatrist", 1),
    ("neurologist", 1),
];

const CONFLICT_NOTE: &str =
    "Declared conflict of interest: family member. Not to be rostered to this client.";
const GUARDIANSHIP_NOTE: &str = "Guardianship order (VCAT)";

/// External organisations and the people at them that clients link to.
#[derive(Default)]
pub struct Networking {
    practices: Vec<Practice>,
    specialists: Vec<(Uuid, &'static str)>,
    coordinators: Vec<(Uuid, Vec<Uuid>)>,
    pharmacies: Vec<Uuid>,
    plan_managers: Vec<Uuid>,
    speech_pathology: Option<Uuid>,
    occupational_therapy: Option<Uuid>,
    physiotherapy: Option<Uuid>,
    public_advocate: Option<Uuid>,
}

struct Practice {
    organisation: Uuid,
    doctors: Vec<Uuid>,
}

/// Workers, plus worker/client pairs that must not be rostered together.
pub struct Staff {
    pub ids: Vec<Uuid>,
    pub conflicts: Vec<(Uuid, Uuid)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Funding {
    Ndis,
    AgedCare,
}

#[derive(Clone, Copy)]
enum Gp {
    None,
    Practice(usize),
    Doctor(usize, usize),
}

#[derive(Clone)]
struct Person {
    id: Uuid,
    name: Name,
    dob: NaiveDate,
}

impl Person {
    fn year(&self) -> i32 {
        self.dob.year()
    }

    fn gender(&self) -> Gender {
        self.name.gender
    }

    fn key(&self) -> (Uuid, Gender) {
        (self.id, self.name.gender)
    }
}

struct Home {
    line1: String,
    suburb: &'static str,
    postcode: &'static str,
    practice: usize,
}

struct Client {
    person: Person,
    funding: Funding,
    gp: Gp,
}

struct ClientSpec {
    person: Person,
    funding: Funding,
    home: usize,
    gp: Gp,
    living: Option<&'static str>,
}

/// How a group of clients shares a family network.
enum Unit {
    Single(usize),
    Couple(usize, usize),
    Siblings(Vec<usize>),
    ParentCarer { parent: usize, child: usize },
}

#[derive(Clone, Copy)]
enum Kin {
    Parent,
    Child,
    Sibling,
    Spouse,
    Partner,
    Grandparent,
    Grandchild,
    Cousin,
    AuntUncle,
    NieceNephew,
}

impl Kin {
    const fn role(self, gender: Gender) -> &'static str {
        match self {
            Self::Parent => gender.pick("mother", "father"),
            Self::Child => gender.pick("daughter", "son"),
            Self::Sibling => gender.pick("sister", "brother"),
            Self::Spouse => gender.pick("wife", "husband"),
            Self::Partner => "partner",
            Self::Grandparent => gender.pick("grandmother", "grandfather"),
            Self::Grandchild => gender.pick("granddaughter", "grandson"),
            Self::Cousin => "cousin",
            Self::AuntUncle => gender.pick("aunt", "uncle"),
            Self::NieceNephew => gender.pick("niece", "nephew"),
        }
    }
}

#[derive(Clone, Copy)]
enum Party {
    Contact(Uuid),
    Organisation(Uuid),
}

impl Party {
    const fn split(self) -> (Option<Uuid>, Option<Uuid>) {
        match self {
            Self::Contact(id) => (Some(id), None),
            Self::Organisation(id) => (None, Some(id)),
        }
    }
}

struct Rel {
    from: Party,
    to: Party,
    from_role: String,
    to_role: String,
    start_date: Option<NaiveDate>,
    notes: Vec<String>,
}

struct ContactRow {
    display: Option<String>,
    email_prefix: &'static str,
    pronouns: Option<&'static str>,
    nickname: bool,
    notes: String,
    details: Value,
}

pub struct People<'a, 'c> {
    tx: &'a mut Transaction<'c, Postgres>,
    rng: &'a mut StdRng,
    organisation: Uuid,
    today: NaiveDate,
    names: NameBook,
    links: Vec<Rel>,
    sequence: u32,
    homes: Vec<Home>,
    clients: Vec<Client>,
    /// Relationship indices from each client to non-client family members.
    family_links: HashMap<usize, Vec<usize>>,
    /// Clients with a family member who is also a client.
    client_family: HashSet<usize>,
}

impl<'a, 'c> People<'a, 'c> {
    pub fn new(
        tx: &'a mut Transaction<'c, Postgres>,
        rng: &'a mut StdRng,
        organisation: Uuid,
    ) -> Self {
        Self {
            tx,
            rng,
            organisation,
            today: Utc::now().date_naive(),
            names: NameBook::default(),
            links: Vec::new(),
            sequence: 0,
            homes: Vec::new(),
            clients: Vec::new(),
            family_links: HashMap::new(),
            client_family: HashSet::new(),
        }
    }

    /// Full names issued more than once across every contact.
    pub fn duplicate_names(&self) -> usize {
        self.names.duplicated()
    }

    // ------------------------------------------------------------------
    // External organisations
    // ------------------------------------------------------------------

    pub async fn external_organisations(&mut self) -> anyhow::Result<Networking> {
        let mut networking = Networking::default();

        for (name, kind) in EXTERNAL_ORGS {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO organisations
                    (id, tenant_id, name, kind, abn, email, phone, is_active)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, true)",
            )
            .bind(id)
            .bind(self.organisation)
            .bind(name)
            .bind(kind)
            .bind(abn(self.rng))
            .bind(format!("info@{}.{EMAIL_DOMAIN}", slug(name)))
            .bind(phone(self.rng))
            .execute(&mut **self.tx)
            .await?;

            let mut representatives = Vec::new();
            for _ in 0..2 {
                let year = self.between(1965, 2000);
                let person = self.community_person(year);
                self.insert_contact(
                    &person,
                    ContactRow {
                        display: None,
                        email_prefix: "contact",
                        pronouns: None,
                        nickname: false,
                        notes: format!("Contact at {name} (demo)."),
                        details: json!({
                            "subtype": "external",
                            "organisation": name
                        }),
                    },
                )
                .await?;
                self.link(
                    Party::Contact(person.id),
                    Party::Organisation(id),
                    "representative",
                    "organisation",
                );
                representatives.push(person.id);
            }

            match *kind {
                "health" => {
                    let count = self.rng.gen_range(2..=4);
                    let mut doctors = Vec::with_capacity(count);
                    for _ in 0..count {
                        doctors.push(self.doctor(name, id, "general_practitioner").await?);
                    }
                    networking.practices.push(Practice {
                        organisation: id,
                        doctors,
                    });
                }
                "medical_specialists" => {
                    for (specialty, count) in SPECIALISTS {
                        for _ in 0..*count {
                            let doctor = self.doctor(name, id, specialty).await?;
                            networking.specialists.push((doctor, specialty));
                        }
                    }
                }
                "support_coordinator" => networking.coordinators.push((id, representatives)),
                "pharmacy" => networking.pharmacies.push(id),
                "plan_manager" => networking.plan_managers.push(id),
                "government" => networking.public_advocate = Some(id),
                "allied_health" => match *name {
                    "Speech Works" => networking.speech_pathology = Some(id),
                    "Occupational Therapy Collective" => {
                        networking.occupational_therapy = Some(id);
                    }
                    "Northside Physiotherapy" => networking.physiotherapy = Some(id),
                    _ => {}
                },
                _ => {}
            }
        }

        Ok(networking)
    }

    async fn doctor(
        &mut self,
        practice: &str,
        practice_id: Uuid,
        specialty: &'static str,
    ) -> anyhow::Result<Uuid> {
        let year = self.between(1958, 1992);
        let person = self.community_person(year);
        let title = specialty.replace('_', " ");
        self.insert_contact(
            &person,
            ContactRow {
                display: Some(format!("Dr {}", person.name.display())),
                email_prefix: "dr",
                pronouns: None,
                nickname: false,
                notes: format!("{} at {practice} (demo).", capitalise(&title)),
                details: json!({
                    "subtype": specialty,
                    "organisation": practice
                }),
            },
        )
        .await?;
        self.link(
            Party::Contact(person.id),
            Party::Organisation(practice_id),
            specialty,
            "practice",
        );
        Ok(person.id)
    }

    // ------------------------------------------------------------------
    // Clients
    // ------------------------------------------------------------------

    pub async fn clients(
        &mut self,
        networking: &Networking,
        fields: &HashMap<String, Uuid>,
    ) -> anyhow::Result<Vec<Uuid>> {
        let mut units = Vec::new();

        self.aged_couples(networking, fields, &mut units).await?;
        self.ageing_parent_carers(networking, fields, &mut units)
            .await?;
        self.sibling_groups(networking, fields, &mut units).await?;
        self.sil_houses(networking, fields, &mut units).await?;
        self.cousins(networking, fields, &mut units).await?;
        self.single_clients(networking, fields, &mut units).await?;

        for unit in &units {
            match unit {
                Unit::Single(index) => {
                    if self.clients[*index].funding == Funding::AgedCare {
                        self.aged_network(&[*index]).await?;
                    } else {
                        self.ndis_network(&[*index], networking).await?;
                    }
                }
                Unit::Couple(first, second) => self.aged_network(&[*first, *second]).await?,
                Unit::Siblings(members) => self.ndis_network(members, networking).await?,
                Unit::ParentCarer { parent, child } => {
                    self.parent_carer_network(*parent, *child).await?;
                }
            }
        }

        self.emergency_contacts().await?;
        self.external_supports(networking);

        Ok(self.clients.iter().map(|client| client.person.id).collect())
    }

    async fn aged_couples(
        &mut self,
        networking: &Networking,
        fields: &HashMap<String, Uuid>,
        units: &mut Vec<Unit>,
    ) -> anyhow::Result<()> {
        for couple in 0..AGED_COUPLES {
            let background = Background::random(self.rng, Cohort::Elder);
            let home = self.new_home();
            // One same-sex couple; the rest are husband and wife.
            let same_sex = couple == 0;
            let first_year = self.aged_birth_year();
            let first_gender = if same_sex {
                Gender::Female
            } else {
                Gender::Male
            };
            let first = self.person(first_gender, first_year, background, None);
            let second_year = (first_year + self.between(-2, 6)).min(self.year_now() - 66);
            let surname = (!same_sex && self.chance(0.92)).then(|| first.name.family.clone());
            let second = self.person(Gender::Female, second_year, background, surname.as_deref());

            let gp = self.choose_gp(home, networking);
            let second_gp = self.shared_gp(gp, home, networking);
            let a = self
                .create_client(
                    ClientSpec {
                        person: first,
                        funding: Funding::AgedCare,
                        home,
                        gp,
                        living: Some("with_spouse"),
                    },
                    fields,
                )
                .await?;
            let b = self
                .create_client(
                    ClientSpec {
                        person: second,
                        funding: Funding::AgedCare,
                        home,
                        gp: second_gp,
                        living: Some("with_spouse"),
                    },
                    fields,
                )
                .await?;

            let kin = if same_sex { Kin::Partner } else { Kin::Spouse };
            let rel = self.kin_link(self.client_key(a), kin, self.client_key(b), kin);
            self.note(rel, "Live together; both receive aged-care services");
            self.client_family.extend([a, b]);
            units.push(Unit::Couple(a, b));
        }
        Ok(())
    }

    async fn ageing_parent_carers(
        &mut self,
        networking: &Networking,
        fields: &HashMap<String, Uuid>,
        units: &mut Vec<Unit>,
    ) -> anyhow::Result<()> {
        for _ in 0..AGEING_PARENT_CARERS {
            let background = Background::random(self.rng, Cohort::Elder);
            let home = self.new_home();
            let parent_age = self.between(70, 92);
            let mut child_age = parent_age - self.between(22, 38);
            if child_age > 62 {
                child_age = 62 - self.between(0, 10);
            }
            let parent_gender = Gender::random(self.rng, 0.7);
            let parent = self.person(
                parent_gender,
                self.year_now() - parent_age,
                background,
                None,
            );
            let child_gender = Gender::random(self.rng, 0.4);
            let surname = parent.name.family.clone();
            let child = self.person(
                child_gender,
                self.year_now() - child_age,
                background,
                Some(&surname),
            );

            let gp = self.choose_gp(home, networking);
            let child_gp = self.shared_gp(gp, home, networking);
            let parent = self
                .create_client(
                    ClientSpec {
                        person: parent,
                        funding: Funding::AgedCare,
                        home,
                        gp,
                        living: Some("with_family"),
                    },
                    fields,
                )
                .await?;
            let child = self
                .create_client(
                    ClientSpec {
                        person: child,
                        funding: Funding::Ndis,
                        home,
                        gp: child_gp,
                        living: Some("with_family"),
                    },
                    fields,
                )
                .await?;

            let rel = self.kin_link(
                self.client_key(child),
                Kin::Child,
                self.client_key(parent),
                Kin::Parent,
            );
            self.note(
                rel,
                "Lives with their ageing parent, who is also their informal carer",
            );
            self.client_family.extend([parent, child]);
            units.push(Unit::ParentCarer { parent, child });
        }
        Ok(())
    }

    async fn sibling_groups(
        &mut self,
        networking: &Networking,
        fields: &HashMap<String, Uuid>,
        units: &mut Vec<Unit>,
    ) -> anyhow::Result<()> {
        for &size in SIBLING_GROUPS {
            let background = Background::random(self.rng, Cohort::General);
            let family_home = self.new_home();
            let gp = self.choose_gp(family_home, networking);
            let eldest_age = self.between(8, 35);
            let mut surname: Option<String> = None;
            let mut age = eldest_age;
            let mut members = Vec::with_capacity(size);

            for position in 0..size {
                if position > 0 {
                    age = (age - self.between(0, 5)).max(5);
                }
                let gender = Gender::random(self.rng, 0.4);
                let person = self.person(
                    gender,
                    self.year_now() - age,
                    background,
                    surname.as_deref(),
                );
                surname.get_or_insert_with(|| person.name.family.clone());
                let moved_out = age >= 28 && position > 0 && self.chance(0.5);
                let home = if moved_out {
                    self.new_home()
                } else {
                    family_home
                };
                let member_gp = self.shared_gp(gp, home, networking);
                let index = self
                    .create_client(
                        ClientSpec {
                            person,
                            funding: Funding::Ndis,
                            home,
                            gp: member_gp,
                            living: (!moved_out && age < 30).then_some("with_family"),
                        },
                        fields,
                    )
                    .await?;
                members.push(index);
            }

            for (position, &first) in members.iter().enumerate() {
                for &second in &members[position + 1..] {
                    self.kin_link(
                        self.client_key(first),
                        Kin::Sibling,
                        self.client_key(second),
                        Kin::Sibling,
                    );
                }
            }
            self.client_family.extend(members.iter().copied());
            units.push(Unit::Siblings(members));
        }
        Ok(())
    }

    async fn sil_houses(
        &mut self,
        networking: &Networking,
        fields: &HashMap<String, Uuid>,
        units: &mut Vec<Unit>,
    ) -> anyhow::Result<()> {
        for &size in SIL_HOUSES {
            let home = self.new_home();
            let house_gp = self.choose_gp(home, networking);
            let mut residents = Vec::with_capacity(size);
            for _ in 0..size {
                let background = Background::random(self.rng, Cohort::General);
                let age = self.between(20, 60);
                let gender = Gender::random(self.rng, 0.4);
                let person = self.person(gender, self.year_now() - age, background, None);
                let gp = self.shared_gp(house_gp, home, networking);
                let index = self
                    .create_client(
                        ClientSpec {
                            person,
                            funding: Funding::Ndis,
                            home,
                            gp,
                            living: Some("supported_independent_living"),
                        },
                        fields,
                    )
                    .await?;
                residents.push(index);
                units.push(Unit::Single(index));
            }

            for (position, &first) in residents.iter().enumerate() {
                for &second in &residents[position + 1..] {
                    let rel = self.link(
                        Party::Contact(self.clients[first].person.id),
                        Party::Contact(self.clients[second].person.id),
                        "housemate",
                        "housemate",
                    );
                    self.note(rel, "Co-residents in a supported independent living house");
                }
            }
        }
        Ok(())
    }

    async fn cousins(
        &mut self,
        networking: &Networking,
        fields: &HashMap<String, Uuid>,
        units: &mut Vec<Unit>,
    ) -> anyhow::Result<()> {
        for _ in 0..COUSIN_PAIRS {
            let background = Background::random(self.rng, Cohort::General);
            let first_age = self.between(10, 40);
            let second_age = (first_age + self.between(-8, 8)).clamp(6, 60);
            let first = self
                .ndis_single(background, first_age, None, networking, fields)
                .await?;
            let surname = self
                .chance(0.4)
                .then(|| self.clients[first].person.name.family.clone());
            let second = self
                .ndis_single(
                    background,
                    second_age,
                    surname.as_deref(),
                    networking,
                    fields,
                )
                .await?;
            self.kin_link(
                self.client_key(first),
                Kin::Cousin,
                self.client_key(second),
                Kin::Cousin,
            );
            self.client_family.extend([first, second]);
            units.extend([Unit::Single(first), Unit::Single(second)]);
        }
        Ok(())
    }

    async fn single_clients(
        &mut self,
        networking: &Networking,
        fields: &HashMap<String, Uuid>,
        units: &mut Vec<Unit>,
    ) -> anyhow::Result<()> {
        let count = |clients: &[Client], funding| {
            clients
                .iter()
                .filter(|client| client.funding == funding)
                .count()
        };
        let mut ndis = count(&self.clients, Funding::Ndis);
        let mut aged = count(&self.clients, Funding::AgedCare);

        while ndis < NDIS_CLIENTS || aged < AGED_CLIENTS {
            // Interleave so client order isn't grouped by funding.
            let take_aged = aged < AGED_CLIENTS && (ndis >= NDIS_CLIENTS || self.chance(0.33));
            let index = if take_aged {
                let background = Background::random(self.rng, Cohort::Elder);
                let gender = Gender::random(self.rng, 0.62);
                let year = self.aged_birth_year();
                let person = self.person(gender, year, background, None);
                let home = self.new_home();
                let gp = self.choose_gp(home, networking);
                aged += 1;
                self.create_client(
                    ClientSpec {
                        person,
                        funding: Funding::AgedCare,
                        home,
                        gp,
                        living: None,
                    },
                    fields,
                )
                .await?
            } else {
                let background = Background::random(self.rng, Cohort::General);
                let age = self.ndis_age();
                ndis += 1;
                self.ndis_single(background, age, None, networking, fields)
                    .await?
            };
            units.push(Unit::Single(index));
        }
        Ok(())
    }

    async fn ndis_single(
        &mut self,
        background: Background,
        age: i32,
        surname: Option<&str>,
        networking: &Networking,
        fields: &HashMap<String, Uuid>,
    ) -> anyhow::Result<usize> {
        let gender = Gender::random(self.rng, 0.4);
        let person = self.person(gender, self.year_now() - age, background, surname);
        let home = self.new_home();
        let gp = self.choose_gp(home, networking);
        self.create_client(
            ClientSpec {
                person,
                funding: Funding::Ndis,
                home,
                gp,
                living: (age < 18).then_some("with_family"),
            },
            fields,
        )
        .await
    }

    async fn create_client(
        &mut self,
        spec: ClientSpec,
        fields: &HashMap<String, Uuid>,
    ) -> anyhow::Result<usize> {
        let ClientSpec {
            person,
            funding,
            home,
            gp,
            living,
        } = spec;
        let home = &self.homes[home];
        let (funding_key, program) = match funding {
            Funding::Ndis => ("ndis", "NDIS participant"),
            Funding::AgedCare => ("aged_care", "Support at Home client"),
        };
        let notes = format!("Lives in {}. {program} (demo).", home.suburb);
        let mut details = json!({
            "address": {
                "line1": home.line1,
                "suburb": home.suburb,
                "state": "VIC",
                "postcode": home.postcode
            },
            "funding": funding_key
        });
        if let Some(living) = living {
            details["living_arrangement"] = json!(living);
        }
        let pronouns = self.pronouns(person.gender(), 0.5);
        self.insert_contact(
            &person,
            ContactRow {
                display: None,
                email_prefix: "client",
                pronouns,
                nickname: true,
                notes,
                details,
            },
        )
        .await?;

        // Being a client is a relationship: the organisation provides care.
        self.link(
            Party::Contact(person.id),
            Party::Organisation(self.organisation),
            "care_recipient",
            "provider",
        );

        if funding == Funding::Ndis {
            let ndis_number = format!("43{:07}", self.rng.gen_range(0..10_000_000u32));
            set_custom_value(
                self.tx,
                self.organisation,
                fields["ndis_number"],
                person.id,
                json!(ndis_number),
            )
            .await?;
            let plan_end = self.today + Duration::days(self.rng.gen_range(90..500));
            set_custom_value(
                self.tx,
                self.organisation,
                fields["plan_end_date"],
                person.id,
                json!(plan_end.to_string()),
            )
            .await?;
        }
        let risk = match self.rng.gen_range(0..10) {
            0..=5 => "low",
            6..=8 => "medium",
            _ => "high",
        };
        set_custom_value(
            self.tx,
            self.organisation,
            fields["risk_level"],
            person.id,
            json!(risk),
        )
        .await?;

        self.clients.push(Client {
            person,
            funding,
            gp,
        });
        Ok(self.clients.len() - 1)
    }

    // ------------------------------------------------------------------
    // Family networks
    // ------------------------------------------------------------------

    /// Parents, siblings, partners and guardians for NDIS participants.
    /// `members` share parents (siblings); the first is the eldest.
    async fn ndis_network(
        &mut self,
        members: &[usize],
        networking: &Networking,
    ) -> anyhow::Result<()> {
        let eldest = self.clients[members[0]].person.clone();
        let age = self.year_now() - eldest.year();
        let background = eldest.name.background;
        let surname = eldest.name.family.clone();
        let minor = age < 18;

        let mother_age = age + self.between(22, 40);
        let father_age = mother_age + self.between(-1, 6);
        let mut parents = Vec::new();
        let mother_involved = minor || self.chance(survival(mother_age) * 0.9);
        if mother_involved {
            let family = self.chance(0.75).then(|| surname.clone());
            let year = self.year_now() - mother_age;
            parents.push(
                self.relative(Gender::Female, year, background, family.as_deref())
                    .await?,
            );
        }
        let father_involved = self.chance(if minor {
            0.75
        } else {
            survival(father_age) * 0.75
        });
        if father_involved {
            let family = self.chance(0.92).then(|| surname.clone());
            let year = self.year_now() - father_age;
            parents.push(
                self.relative(Gender::Male, year, background, family.as_deref())
                    .await?,
            );
        }

        for (position, parent) in parents.iter().enumerate() {
            for &member in members {
                let rel = self.kin_link(
                    self.client_key(member),
                    Kin::Child,
                    parent.key(),
                    Kin::Parent,
                );
                if position == 0 && self.client_age(member) < 30 {
                    self.note(rel, "Primary carer");
                }
                self.family_links.entry(member).or_default().push(rel);
            }
        }

        // An adult participant may have a parent appointed as guardian.
        let guardian_chance = if age < 30 { 0.25 } else { 0.1 };
        if !minor && !parents.is_empty() && self.chance(guardian_chance) {
            let guardian = parents[0].id;
            for &member in members {
                if self.client_age(member) >= 18 {
                    let rel = self.link(
                        Party::Contact(self.clients[member].person.id),
                        Party::Contact(guardian),
                        "person_supported",
                        "guardian",
                    );
                    self.note(rel, GUARDIANSHIP_NOTE);
                }
            }
        }

        // A brother or sister who isn't a client.
        let sibling_chance = if members.len() > 1 { 0.25 } else { 0.45 };
        if self.chance(sibling_chance) {
            let sibling_age = age + self.between(-8, 8);
            if sibling_age >= 18 {
                let gender = Gender::random(self.rng, 0.5);
                let keeps_name = gender == Gender::Male || self.chance(0.6);
                let family = keeps_name.then(|| surname.clone());
                let year = self.year_now() - sibling_age;
                let sibling = self
                    .relative(gender, year, background, family.as_deref())
                    .await?;
                for &member in members {
                    let rel = self.kin_link(
                        self.client_key(member),
                        Kin::Sibling,
                        sibling.key(),
                        Kin::Sibling,
                    );
                    self.family_links.entry(member).or_default().push(rel);
                }
            }
        }

        if let [only] = members {
            let only = *only;
            if age >= 25 && self.chance(0.18) {
                let gender = if self.chance(0.9) {
                    eldest.gender().opposite()
                } else {
                    eldest.gender()
                };
                let year = eldest.year() + self.between(-5, 5);
                let partner_background = if self.chance(0.8) {
                    background
                } else {
                    Background::random(self.rng, Cohort::General)
                };
                let partner = self
                    .relative(gender, year, partner_background, None)
                    .await?;
                let rel = self.kin_link(
                    self.client_key(only),
                    Kin::Partner,
                    partner.key(),
                    Kin::Partner,
                );
                self.family_links.entry(only).or_default().push(rel);
            }
            if age >= 40 && self.chance(0.1) {
                let child_age = age - self.between(20, 32);
                if child_age >= 18 {
                    let gender = Gender::random(self.rng, 0.5);
                    let year = self.year_now() - child_age;
                    let child = self
                        .relative(gender, year, background, Some(&surname))
                        .await?;
                    let rel =
                        self.kin_link(self.client_key(only), Kin::Parent, child.key(), Kin::Child);
                    self.family_links.entry(only).or_default().push(rel);
                }
            }
        }

        // The Public Advocate steps in for adults with no family to decide.
        if let Some(advocate) = networking.public_advocate {
            for &member in members {
                if self.client_age(member) < 18 {
                    continue;
                }
                let no_family = self.family_links.get(&member).is_none_or(Vec::is_empty);
                if self.chance(if no_family { 0.5 } else { 0.04 }) {
                    let rel = self.link(
                        Party::Contact(self.clients[member].person.id),
                        Party::Organisation(advocate),
                        "represented_person",
                        "guardian",
                    );
                    self.note(rel, GUARDIANSHIP_NOTE);
                }
            }
        }

        Ok(())
    }

    /// Spouse, adult children and grandchildren for aged-care clients.
    /// `members` is one client or a couple who share their children.
    async fn aged_network(&mut self, members: &[usize]) -> anyhow::Result<()> {
        let principal = self.clients[members[0]].person.clone();
        let age = self.year_now() - principal.year();
        let background = principal.name.background;
        let household_name = principal.name.family.clone();

        if let [only] = members {
            if self.chance(0.4) {
                let gender = principal.gender().opposite();
                let keeps_name = principal.gender() == Gender::Female || self.chance(0.92);
                let family = keeps_name.then(|| household_name.clone());
                let spouse_age = (age + self.between(-4, 5)).max(62);
                let year = self.year_now() - spouse_age;
                let spouse = self
                    .relative(gender, year, background, family.as_deref())
                    .await?;
                let rel = self.kin_link(
                    self.client_key(*only),
                    Kin::Spouse,
                    spouse.key(),
                    Kin::Spouse,
                );
                self.note(rel, "Lives at the same address");
                self.family_links.entry(*only).or_default().push(rel);
            }
        }

        let children = match self.rng.gen_range(0..100) {
            0..=14 => 0,
            15..=44 => 1,
            45..=79 => 2,
            _ => 3,
        };
        let mut kids = Vec::with_capacity(children);
        for _ in 0..children {
            let gender = Gender::random(self.rng, 0.5);
            let keeps_name = gender == Gender::Male || self.chance(0.45);
            let family = keeps_name.then(|| household_name.clone());
            let kid_age = (age - self.between(24, 40)).max(22);
            let year = self.year_now() - kid_age;
            let kid = self
                .relative(gender, year, background, family.as_deref())
                .await?;
            for &member in members {
                let rel =
                    self.kin_link(self.client_key(member), Kin::Parent, kid.key(), Kin::Child);
                self.family_links.entry(member).or_default().push(rel);
            }
            kids.push(kid);
        }

        if !kids.is_empty() && self.chance(0.45) {
            let attorney = kids[self.rng.gen_range(0..kids.len())].id;
            for &member in members {
                self.link(
                    Party::Contact(self.clients[member].person.id),
                    Party::Contact(attorney),
                    "principal",
                    "enduring_power_of_attorney",
                );
            }
        }

        if let Some(kid) = kids.first().cloned() {
            if self.chance(0.12) {
                let grandchild_age = self.year_now() - kid.year() - self.between(22, 34);
                if grandchild_age >= 18 {
                    let gender = Gender::random(self.rng, 0.5);
                    let family = self.chance(0.7).then(|| kid.name.family.clone());
                    let year = self.year_now() - grandchild_age;
                    let grandchild = self
                        .relative(gender, year, background, family.as_deref())
                        .await?;
                    for &member in members {
                        let rel = self.kin_link(
                            self.client_key(member),
                            Kin::Grandparent,
                            grandchild.key(),
                            Kin::Grandchild,
                        );
                        self.family_links.entry(member).or_default().push(rel);
                    }
                }
            }
        }

        Ok(())
    }

    /// An ageing parent (aged care) still caring for an adult child (NDIS):
    /// the other parent and the brothers and sisters are shared.
    async fn parent_carer_network(&mut self, parent: usize, child: usize) -> anyhow::Result<()> {
        let parent_person = self.clients[parent].person.clone();
        let child_person = self.clients[child].person.clone();
        let background = parent_person.name.background;
        let surname = child_person.name.family.clone();

        if self.chance(0.3) {
            let year = (parent_person.year() + self.between(-5, 3)).min(self.year_now() - 66);
            let other = self
                .relative(
                    parent_person.gender().opposite(),
                    year,
                    background,
                    Some(&surname),
                )
                .await?;
            let rel = self.kin_link(
                self.client_key(parent),
                Kin::Spouse,
                other.key(),
                Kin::Spouse,
            );
            self.family_links.entry(parent).or_default().push(rel);
            let rel = self.kin_link(self.client_key(child), Kin::Child, other.key(), Kin::Parent);
            self.family_links.entry(child).or_default().push(rel);
        }

        let mut siblings = Vec::new();
        for _ in 0..self.between(0, 2) {
            let gender = Gender::random(self.rng, 0.5);
            let keeps_name = gender == Gender::Male || self.chance(0.45);
            let family = keeps_name.then(|| surname.clone());
            let latest_year = parent_person.year() + 40;
            let year = (child_person.year() + self.between(-8, 8))
                .clamp(parent_person.year() + 20, latest_year);
            let sibling = self
                .relative(gender, year, background, family.as_deref())
                .await?;
            let rel = self.kin_link(
                self.client_key(parent),
                Kin::Parent,
                sibling.key(),
                Kin::Child,
            );
            self.family_links.entry(parent).or_default().push(rel);
            let rel = self.kin_link(
                self.client_key(child),
                Kin::Sibling,
                sibling.key(),
                Kin::Sibling,
            );
            self.family_links.entry(child).or_default().push(rel);
            siblings.push(sibling);
        }

        if let Some(sibling) = siblings.first().map(|sibling| sibling.id) {
            if self.chance(0.3) {
                let rel = self.link(
                    Party::Contact(child_person.id),
                    Party::Contact(sibling),
                    "person_supported",
                    "guardian",
                );
                self.note(rel, GUARDIANSHIP_NOTE);
            }
            if self.chance(0.5) {
                self.link(
                    Party::Contact(parent_person.id),
                    Party::Contact(sibling),
                    "principal",
                    "enduring_power_of_attorney",
                );
            }
        }

        Ok(())
    }

    /// The first family member is the primary emergency contact. Clients
    /// without family (and a few with) get a neighbour or family friend.
    async fn emergency_contacts(&mut self) -> anyhow::Result<()> {
        for index in 0..self.clients.len() {
            let first = self
                .family_links
                .get(&index)
                .and_then(|links| links.first().copied());
            let needs_other = match first {
                Some(rel) => {
                    self.note(rel, "Primary emergency contact");
                    self.chance(0.1)
                }
                None => !self.client_family.contains(&index) || self.chance(0.5),
            };
            if !needs_other {
                continue;
            }

            let year = self.between(1945, 1995);
            let person = self.community_person(year);
            let label = if self.chance(0.5) {
                "Neighbour"
            } else {
                "Family friend"
            };
            self.insert_contact(
                &person,
                ContactRow {
                    display: None,
                    email_prefix: "emergency",
                    pronouns: None,
                    nickname: false,
                    notes: format!("{label} and emergency contact (demo)."),
                    details: json!({ "subtype": "emergency_contact" }),
                },
            )
            .await?;
            let rel = self.link(
                Party::Contact(self.clients[index].person.id),
                Party::Contact(person.id),
                "person_supported",
                "emergency_contact",
            );
            self.note(rel, label);
        }
        Ok(())
    }

    /// GPs, specialists, coordinators, plan managers, pharmacies and allied
    /// health, chosen by funding and age.
    fn external_supports(&mut self, networking: &Networking) {
        for index in 0..self.clients.len() {
            let id = Party::Contact(self.clients[index].person.id);
            let funding = self.clients[index].funding;
            let gp = self.clients[index].gp;
            let age = self.client_age(index);

            match gp {
                Gp::Doctor(practice, doctor) => {
                    let doctor = networking.practices[practice].doctors[doctor];
                    self.link(
                        id,
                        Party::Contact(doctor),
                        "patient",
                        "general_practitioner",
                    );
                }
                Gp::Practice(practice) => {
                    let practice = networking.practices[practice].organisation;
                    self.link(
                        id,
                        Party::Organisation(practice),
                        "patient",
                        "general_practitioner",
                    );
                }
                Gp::None => {}
            }

            let specialty = match funding {
                Funding::Ndis if age < 18 && self.chance(0.6) => Some("paediatrician"),
                Funding::Ndis if age >= 18 && self.chance(0.12) => Some("psychiatrist"),
                Funding::Ndis if self.chance(0.05) => Some("neurologist"),
                Funding::AgedCare if age >= 75 && self.chance(0.25) => Some("geriatrician"),
                Funding::AgedCare if self.chance(0.04) => Some("neurologist"),
                _ => None,
            };
            if let Some(specialty) = specialty {
                let options: Vec<Uuid> = networking
                    .specialists
                    .iter()
                    .filter(|(_, kind)| *kind == specialty)
                    .map(|(doctor, _)| *doctor)
                    .collect();
                if !options.is_empty() {
                    let doctor = options[self.rng.gen_range(0..options.len())];
                    self.link(id, Party::Contact(doctor), "patient", specialty);
                }
            }

            match funding {
                Funding::Ndis => {
                    if !networking.coordinators.is_empty() && self.chance(0.6) {
                        let (organisation, contacts) = &networking.coordinators
                            [self.rng.gen_range(0..networking.coordinators.len())];
                        self.link(
                            id,
                            Party::Organisation(*organisation),
                            "participant",
                            "support_coordinator",
                        );
                        if !contacts.is_empty() && self.chance(0.7) {
                            let coordinator = contacts[self.rng.gen_range(0..contacts.len())];
                            self.link(
                                id,
                                Party::Contact(coordinator),
                                "participant",
                                "support_coordinator",
                            );
                        }
                    }
                    if self.chance(0.55) {
                        self.link_random(
                            id,
                            &networking.plan_managers,
                            "participant",
                            "plan_manager",
                        );
                    }
                    if self.chance(0.3) {
                        self.link_random(id, &networking.pharmacies, "customer", "pharmacy");
                    }
                    if age < 18 && self.chance(0.35) {
                        self.link_optional(id, networking.speech_pathology, "speech_pathology");
                    }
                    if self.chance(0.2) {
                        self.link_optional(
                            id,
                            networking.occupational_therapy,
                            "occupational_therapy",
                        );
                    }
                }
                Funding::AgedCare => {
                    if self.chance(0.7) {
                        self.link_random(id, &networking.pharmacies, "customer", "pharmacy");
                    }
                    if self.chance(0.2) {
                        self.link_optional(id, networking.physiotherapy, "physiotherapy");
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Staff
    // ------------------------------------------------------------------

    pub async fn staff(&mut self, fields: &HashMap<String, Uuid>) -> anyhow::Result<Staff> {
        let mut ids = Vec::with_capacity(STAFF);
        let mut workers: Vec<Person> = Vec::with_capacity(STAFF);
        let mut conflicts = Vec::new();

        let conflict_count = 5;
        let related = WORKER_COUPLES + WORKER_SIBLINGS + WORKER_PARENT_CHILD + WORKER_COUSINS;
        let base = STAFF - related - conflict_count;

        for _ in 0..base {
            let background = Background::random(self.rng, Cohort::Workforce);
            let gender = Gender::random(self.rng, 0.76);
            let year = self.between(1962, 2004);
            let person = self.person(gender, year, background, None);
            self.insert_worker(&person, ids.len(), fields).await?;
            ids.push(person.id);
            workers.push(person);
        }

        let mut taken = HashSet::new();

        for couple in 0..WORKER_COUPLES {
            let Some(index) = self.pick_worker(&workers, &mut taken, |w| w.year() <= 1998) else {
                continue;
            };
            let worker = workers[index].clone();
            let same_sex = couple == 0;
            let gender = if same_sex {
                worker.gender()
            } else {
                worker.gender().opposite()
            };
            let year = (worker.year() + self.between(-5, 5)).clamp(1962, 2002);
            let shares_name = !same_sex && self.chance(0.55);
            let family = shares_name.then(|| worker.name.family.clone());
            let spouse = self.person(gender, year, worker.name.background, family.as_deref());
            self.insert_worker(&spouse, ids.len(), fields).await?;
            ids.push(spouse.id);
            let kin = if same_sex || !(shares_name || self.chance(0.6)) {
                Kin::Partner
            } else {
                Kin::Spouse
            };
            self.kin_link(worker.key(), kin, spouse.key(), kin);
        }

        for _ in 0..WORKER_SIBLINGS {
            let Some(index) = self.pick_worker(&workers, &mut taken, |_| true) else {
                continue;
            };
            let worker = workers[index].clone();
            let year = (worker.year() + self.between(-6, 6)).clamp(1962, 2004);
            let gender = Gender::random(self.rng, 0.7);
            let sibling = self.person(
                gender,
                year,
                worker.name.background,
                Some(&worker.name.family),
            );
            self.insert_worker(&sibling, ids.len(), fields).await?;
            ids.push(sibling.id);
            self.kin_link(worker.key(), Kin::Sibling, sibling.key(), Kin::Sibling);
        }

        for _ in 0..WORKER_PARENT_CHILD {
            let Some(index) = self.pick_worker(&workers, &mut taken, |w| w.year() <= 1977) else {
                continue;
            };
            let parent = workers[index].clone();
            let year = (parent.year() + self.between(22, 30)).min(2004);
            let gender = Gender::random(self.rng, 0.7);
            let keeps_name = parent.gender() == Gender::Male || self.chance(0.7);
            let family = keeps_name.then(|| parent.name.family.clone());
            let child = self.person(gender, year, parent.name.background, family.as_deref());
            self.insert_worker(&child, ids.len(), fields).await?;
            ids.push(child.id);
            self.kin_link(parent.key(), Kin::Parent, child.key(), Kin::Child);
        }

        for _ in 0..WORKER_COUSINS {
            let Some(index) = self.pick_worker(&workers, &mut taken, |_| true) else {
                continue;
            };
            let worker = workers[index].clone();
            let year = (worker.year() + self.between(-8, 8)).clamp(1962, 2004);
            let gender = Gender::random(self.rng, 0.7);
            let family = self.chance(0.4).then(|| worker.name.family.clone());
            let cousin = self.person(gender, year, worker.name.background, family.as_deref());
            self.insert_worker(&cousin, ids.len(), fields).await?;
            ids.push(cousin.id);
            self.kin_link(worker.key(), Kin::Cousin, cousin.key(), Kin::Cousin);
        }

        // Workers who are family of a client: declared and never rostered.
        let year_now = self.year_now();
        let conflict_plan: [(Funding, std::ops::RangeInclusive<i32>, Kin, Kin); 5] = [
            (Funding::Ndis, 20..=45, Kin::Sibling, Kin::Sibling),
            (Funding::AgedCare, 65..=88, Kin::Child, Kin::Parent),
            (Funding::AgedCare, 65..=88, Kin::Child, Kin::Parent),
            (Funding::Ndis, 5..=17, Kin::AuntUncle, Kin::NieceNephew),
            (Funding::Ndis, 18..=40, Kin::Cousin, Kin::Cousin),
        ];
        let mut chosen = HashSet::new();
        for (funding, ages, worker_kin, client_kin) in conflict_plan {
            let candidates: Vec<usize> = (0..self.clients.len())
                .filter(|index| {
                    let client = &self.clients[*index];
                    client.funding == funding
                        && ages.contains(&(year_now - client.person.year()))
                        && !chosen.contains(index)
                })
                .collect();
            if candidates.is_empty() {
                continue;
            }
            let index = candidates[self.rng.gen_range(0..candidates.len())];
            chosen.insert(index);
            let client = self.clients[index].person.clone();

            let year = match worker_kin {
                Kin::Child => client.year() + self.between(25, 38),
                Kin::AuntUncle => client.year() - self.between(25, 40),
                _ => client.year() + self.between(-6, 6),
            }
            .clamp(1962, 2004);
            let gender = Gender::random(self.rng, 0.76);
            let keeps_name = match worker_kin {
                Kin::Sibling => true,
                Kin::Child => gender == Gender::Male || self.chance(0.5),
                _ => self.chance(0.4),
            };
            let family = keeps_name.then(|| client.name.family.clone());
            let worker = self.person(gender, year, client.name.background, family.as_deref());
            self.insert_worker(&worker, ids.len(), fields).await?;
            ids.push(worker.id);
            let rel = self.kin_link(worker.key(), worker_kin, client.key(), client_kin);
            self.note(rel, CONFLICT_NOTE);
            conflicts.push((worker.id, client.id));
        }

        Ok(Staff { ids, conflicts })
    }

    fn pick_worker(
        &mut self,
        workers: &[Person],
        taken: &mut HashSet<usize>,
        accept: impl Fn(&Person) -> bool,
    ) -> Option<usize> {
        for _ in 0..500 {
            let index = self.rng.gen_range(0..workers.len());
            if !taken.contains(&index) && accept(&workers[index]) {
                taken.insert(index);
                return Some(index);
            }
        }
        None
    }

    async fn insert_worker(
        &mut self,
        person: &Person,
        index: usize,
        fields: &HashMap<String, Uuid>,
    ) -> anyhow::Result<()> {
        let start = NaiveDate::from_ymd_opt(2015, 1, 5).expect("valid date")
            + Duration::days(self.rng.gen_range(0..4_200));
        let start = start.min(self.today - Duration::days(30));
        let employment = if self.chance(0.35) {
            "casual"
        } else {
            "permanent"
        };
        let pronouns = self.pronouns(person.gender(), 0.5);
        self.insert_contact(
            person,
            ContactRow {
                display: None,
                email_prefix: "staff",
                pronouns,
                nickname: true,
                notes: "Support worker (demo).".to_owned(),
                details: json!({
                    "employment": {
                        "type": employment,
                        "start_date": start.to_string()
                    }
                }),
            },
        )
        .await?;

        let rel = self.link(
            Party::Contact(person.id),
            Party::Organisation(self.organisation),
            "employee",
            "employer",
        );
        self.links[rel].start_date = Some(start);

        set_custom_value(
            self.tx,
            self.organisation,
            fields["employee_number"],
            person.id,
            json!(format!("EMP-{:04}", index + 1)),
        )
        .await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Building blocks
    // ------------------------------------------------------------------

    /// Writes every collected relationship.
    pub async fn flush(&mut self) -> anyhow::Result<()> {
        for rel in self.links.drain(..) {
            let (from_contact, from_organisation) = rel.from.split();
            let (to_contact, to_organisation) = rel.to.split();
            let notes = (!rel.notes.is_empty()).then(|| rel.notes.join("; "));
            sqlx::query(
                "INSERT INTO relationships
                    (id, organisation_id, from_contact_id, from_organisation_id,
                     to_contact_id, to_organisation_id, from_role, to_role, start_date, notes)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(Uuid::now_v7())
            .bind(self.organisation)
            .bind(from_contact)
            .bind(from_organisation)
            .bind(to_contact)
            .bind(to_organisation)
            .bind(&rel.from_role)
            .bind(&rel.to_role)
            .bind(rel.start_date)
            .bind(notes)
            .execute(&mut **self.tx)
            .await?;
        }
        Ok(())
    }

    fn link(&mut self, from: Party, to: Party, from_role: &str, to_role: &str) -> usize {
        self.links.push(Rel {
            from,
            to,
            from_role: from_role.to_owned(),
            to_role: to_role.to_owned(),
            start_date: None,
            notes: Vec::new(),
        });
        self.links.len() - 1
    }

    /// "<a> is <a_kin> of <b>", e.g. a client who is the daughter of her mother.
    fn kin_link(&mut self, a: (Uuid, Gender), a_kin: Kin, b: (Uuid, Gender), b_kin: Kin) -> usize {
        self.link(
            Party::Contact(a.0),
            Party::Contact(b.0),
            a_kin.role(a.1),
            b_kin.role(b.1),
        )
    }

    fn link_random(&mut self, from: Party, options: &[Uuid], from_role: &str, to_role: &str) {
        if !options.is_empty() {
            let organisation = options[self.rng.gen_range(0..options.len())];
            self.link(from, Party::Organisation(organisation), from_role, to_role);
        }
    }

    fn link_optional(&mut self, from: Party, organisation: Option<Uuid>, to_role: &str) {
        if let Some(organisation) = organisation {
            self.link(from, Party::Organisation(organisation), "client", to_role);
        }
    }

    fn note(&mut self, rel: usize, text: &str) {
        self.links[rel].notes.push(text.to_owned());
    }

    fn client_key(&self, index: usize) -> (Uuid, Gender) {
        self.clients[index].person.key()
    }

    fn client_age(&self, index: usize) -> i32 {
        self.year_now() - self.clients[index].person.year()
    }

    fn year_now(&self) -> i32 {
        self.today.year()
    }

    fn chance(&mut self, probability: f64) -> bool {
        self.rng.gen_bool(probability.clamp(0.0, 1.0))
    }

    fn between(&mut self, low: i32, high: i32) -> i32 {
        self.rng.gen_range(low..=high)
    }

    /// NDIS participants: about a quarter children, the rest adults under 65.
    fn ndis_age(&mut self) -> i32 {
        match self.rng.gen_range(0..100) {
            0..=24 => self.between(5, 17),
            25..=54 => self.between(18, 29),
            _ => self.between(30, 64),
        }
    }

    /// Aged-care clients: 65 to 99, most in their late seventies and eighties.
    fn aged_birth_year(&mut self) -> i32 {
        let age = 66 + self.between(0, 16) + self.between(0, 17);
        self.year_now() - age
    }

    fn pronouns(&mut self, gender: Gender, share: f64) -> Option<&'static str> {
        if self.chance(0.02) {
            Some("they/them")
        } else if self.chance(share) {
            Some(gender.pronouns())
        } else {
            None
        }
    }

    fn person(
        &mut self,
        gender: Gender,
        year: i32,
        background: Background,
        family: Option<&str>,
    ) -> Person {
        let name = self.names.name(self.rng, gender, year, background, family);
        let day = self.rng.gen_range(1..=365);
        let dob = NaiveDate::from_yo_opt(year, day)
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, 6, 30).expect("valid date"))
            .min(self.today - Duration::days(1));
        Person {
            id: Uuid::now_v7(),
            name,
            dob,
        }
    }

    fn community_person(&mut self, year: i32) -> Person {
        let background = Background::random(self.rng, Cohort::General);
        let gender = Gender::random(self.rng, 0.55);
        self.person(gender, year, background, None)
    }

    /// A family member who is not a client.
    async fn relative(
        &mut self,
        gender: Gender,
        year: i32,
        background: Background,
        family: Option<&str>,
    ) -> anyhow::Result<Person> {
        let person = self.person(gender, year, background, family);
        self.insert_contact(
            &person,
            ContactRow {
                display: None,
                email_prefix: "family",
                pronouns: None,
                nickname: true,
                notes: "Family member (demo).".to_owned(),
                details: json!({ "subtype": "family" }),
            },
        )
        .await?;
        Ok(person)
    }

    fn new_home(&mut self) -> usize {
        let (suburb, postcode, practice) = SUBURBS[self.rng.gen_range(0..SUBURBS.len())];
        let street = STREETS[self.rng.gen_range(0..STREETS.len())];
        let number = self.rng.gen_range(1..220);
        let line1 = if self.chance(0.15) {
            format!("{}/{number} {street} Street", self.rng.gen_range(1..12))
        } else {
            format!("{number} {street} Street")
        };
        self.homes.push(Home {
            line1,
            suburb,
            postcode,
            practice,
        });
        self.homes.len() - 1
    }

    /// Most people see a GP near home; some kept the one they had before
    /// moving, and a few have none recorded.
    fn choose_gp(&mut self, home: usize, networking: &Networking) -> Gp {
        if networking.practices.is_empty() || self.chance(0.06) {
            return Gp::None;
        }
        let local = self.homes[home]
            .practice
            .min(networking.practices.len() - 1);
        let practice = if self.chance(0.8) {
            local
        } else {
            self.rng.gen_range(0..networking.practices.len())
        };
        let doctors = networking.practices[practice].doctors.len();
        if doctors > 0 && self.chance(0.85) {
            Gp::Doctor(practice, self.rng.gen_range(0..doctors))
        } else {
            Gp::Practice(practice)
        }
    }

    /// Households usually share a GP.
    fn shared_gp(&mut self, other: Gp, home: usize, networking: &Networking) -> Gp {
        if self.chance(0.85) {
            other
        } else {
            self.choose_gp(home, networking)
        }
    }

    async fn insert_contact(&mut self, person: &Person, row: ContactRow) -> anyhow::Result<()> {
        self.sequence += 1;
        let email = format!(
            "{}.{}.{}{}@{EMAIL_DOMAIN}",
            row.email_prefix,
            slug(&person.name.given),
            slug(&person.name.family),
            self.sequence
        );
        let preferred = if row.nickname {
            nickname(&person.name.given).filter(|_| self.rng.gen_bool(0.6))
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO contacts
                (id, organisation_id, display_name, given_name, family_name,
                 preferred_name, pronouns, date_of_birth, email, phone, notes, details)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(person.id)
        .bind(self.organisation)
        .bind(row.display.unwrap_or_else(|| person.name.display()))
        .bind(&person.name.given)
        .bind(&person.name.family)
        .bind(preferred)
        .bind(row.pronouns)
        .bind(person.dob)
        .bind(email)
        .bind(phone(self.rng))
        .bind(row.notes)
        .bind(row.details)
        .execute(&mut **self.tx)
        .await?;
        Ok(())
    }
}

/// Rough chance that a parent of the given age is alive and involved.
fn survival(age: i32) -> f64 {
    match age {
        ..=59 => 0.98,
        60..=69 => 0.93,
        70..=79 => 0.8,
        80..=89 => 0.5,
        _ => 0.15,
    }
}

fn slug(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn capitalise(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
