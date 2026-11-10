//! Capability-based authorisation.
//!
//! Sets of capabilities are stored per organisation and assigned to users. The
//! owner bypasses checks. The DB catalogue `capabilities` mirrors this list;
//! keep both in sync.

/// A single permission. Never check role names in authorisation code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Capability {
    ContactsRead = 0,
    ContactsWrite = 1,
    RelationshipsRead = 2,
    RelationshipsWrite = 3,
    OrganisationsRead = 4,
    OrganisationsWrite = 5,
    DivisionsRead = 6,
    DivisionsWrite = 7,
    CostCentresRead = 8,
    CostCentresWrite = 9,
    ServicesRead = 10,
    ServicesWrite = 11,
    RatesRead = 12,
    RatesWrite = 13,
    CustomFieldsManage = 14,
    RostersRead = 15,
    RostersWrite = 16,
    RostersApprove = 17,
    FinanceRead = 18,
    FinanceWrite = 19,
    HrRead = 20,
    HrWrite = 21,
    UsersManage = 22,
    PermissionSetsManage = 23,
    SettingsManage = 24,
    AuditRead = 25,
    ReportsRead = 26,
    DataImport = 27,
    IntegrationsManage = 28,
    CommsManage = 29,
    SchemaView = 30,
    RelationshipsRemove = 31,
    RelationshipsIncidentNotes = 32,
    RelationshipTypesManage = 33,
    WorkerQualitiesManage = 34,
}

impl Capability {
    pub const ALL: [Self; 35] = [
        Self::ContactsRead,
        Self::ContactsWrite,
        Self::RelationshipsRead,
        Self::RelationshipsWrite,
        Self::OrganisationsRead,
        Self::OrganisationsWrite,
        Self::DivisionsRead,
        Self::DivisionsWrite,
        Self::CostCentresRead,
        Self::CostCentresWrite,
        Self::ServicesRead,
        Self::ServicesWrite,
        Self::RatesRead,
        Self::RatesWrite,
        Self::CustomFieldsManage,
        Self::RostersRead,
        Self::RostersWrite,
        Self::RostersApprove,
        Self::FinanceRead,
        Self::FinanceWrite,
        Self::HrRead,
        Self::HrWrite,
        Self::UsersManage,
        Self::PermissionSetsManage,
        Self::SettingsManage,
        Self::AuditRead,
        Self::ReportsRead,
        Self::DataImport,
        Self::IntegrationsManage,
        Self::CommsManage,
        Self::SchemaView,
        Self::RelationshipsRemove,
        Self::RelationshipsIncidentNotes,
        Self::RelationshipTypesManage,
        Self::WorkerQualitiesManage,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContactsRead => "contacts.read",
            Self::ContactsWrite => "contacts.write",
            Self::RelationshipsRead => "relationships.read",
            Self::RelationshipsWrite => "relationships.write",
            Self::OrganisationsRead => "organisations.read",
            Self::OrganisationsWrite => "organisations.write",
            Self::DivisionsRead => "divisions.read",
            Self::DivisionsWrite => "divisions.write",
            Self::CostCentresRead => "cost_centres.read",
            Self::CostCentresWrite => "cost_centres.write",
            Self::ServicesRead => "services.read",
            Self::ServicesWrite => "services.write",
            Self::RatesRead => "rates.read",
            Self::RatesWrite => "rates.write",
            Self::CustomFieldsManage => "custom_fields.manage",
            Self::RostersRead => "rosters.read",
            Self::RostersWrite => "rosters.write",
            Self::RostersApprove => "rosters.approve",
            Self::FinanceRead => "finance.read",
            Self::FinanceWrite => "finance.write",
            Self::HrRead => "hr.read",
            Self::HrWrite => "hr.write",
            Self::UsersManage => "users.manage",
            Self::PermissionSetsManage => "permission_sets.manage",
            Self::SettingsManage => "settings.manage",
            Self::AuditRead => "audit.read",
            Self::ReportsRead => "reports.read",
            Self::DataImport => "data.import",
            Self::IntegrationsManage => "integrations.manage",
            Self::CommsManage => "comms.manage",
            Self::SchemaView => "schema.view",
            Self::RelationshipsRemove => "relationships.remove",
            Self::RelationshipsIncidentNotes => "relationships.incident_notes",
            Self::RelationshipTypesManage => "relationship_types.manage",
            Self::WorkerQualitiesManage => "worker_qualities.manage",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ContactsRead => "View people",
            Self::ContactsWrite => "Edit people",
            Self::RelationshipsRead => "View relationships",
            Self::RelationshipsWrite => "Edit relationships",
            Self::OrganisationsRead => "View organisations",
            Self::OrganisationsWrite => "Edit organisations",
            Self::DivisionsRead => "View divisions",
            Self::DivisionsWrite => "Edit divisions",
            Self::CostCentresRead => "View cost centres",
            Self::CostCentresWrite => "Edit cost centres",
            Self::ServicesRead => "View services",
            Self::ServicesWrite => "Edit services",
            Self::RatesRead => "View rates",
            Self::RatesWrite => "Edit rates",
            Self::CustomFieldsManage => "Manage custom fields",
            Self::RostersRead => "View rosters",
            Self::RostersWrite => "Edit rosters",
            Self::RostersApprove => "Approve rosters",
            Self::FinanceRead => "View finances",
            Self::FinanceWrite => "Edit finances",
            Self::HrRead => "View HR records",
            Self::HrWrite => "Edit HR records",
            Self::UsersManage => "Manage users",
            Self::PermissionSetsManage => "Manage permission sets",
            Self::SettingsManage => "Manage settings",
            Self::AuditRead => "View audit log",
            Self::ReportsRead => "View reports",
            Self::DataImport => "Import data",
            Self::IntegrationsManage => "Manage integrations",
            Self::CommsManage => "Manage communications",
            Self::SchemaView => "View schema",
            Self::RelationshipsRemove => "Remove relationships",
            Self::RelationshipsIncidentNotes => "View incident notes",
            Self::RelationshipTypesManage => "Manage relationship types",
            Self::WorkerQualitiesManage => "Manage worker qualities",
        }
    }

    #[must_use]
    pub const fn category(self) -> &'static str {
        match self {
            Self::ContactsRead
            | Self::ContactsWrite
            | Self::RelationshipsRead
            | Self::RelationshipsWrite
            | Self::RelationshipsRemove
            | Self::RelationshipsIncidentNotes
            | Self::RelationshipTypesManage
            | Self::WorkerQualitiesManage
            | Self::OrganisationsRead
            | Self::OrganisationsWrite => "People",
            Self::DivisionsRead
            | Self::DivisionsWrite
            | Self::CostCentresRead
            | Self::CostCentresWrite
            | Self::ServicesRead
            | Self::ServicesWrite
            | Self::RatesRead
            | Self::RatesWrite
            | Self::CustomFieldsManage => "Catalogue",
            Self::RostersRead | Self::RostersWrite | Self::RostersApprove => "Rostering",
            Self::FinanceRead | Self::FinanceWrite => "Finance",
            Self::HrRead | Self::HrWrite => "Human resources",
            Self::UsersManage
            | Self::PermissionSetsManage
            | Self::SettingsManage
            | Self::AuditRead
            | Self::ReportsRead
            | Self::DataImport
            | Self::IntegrationsManage
            | Self::CommsManage
            | Self::SchemaView => "Administration",
        }
    }

    #[must_use]
    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.as_str() == key)
    }
}

/// A cheap bit set of capabilities.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapabilitySet(u64);

impl CapabilitySet {
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn all() -> Self {
        Self((1u64 << Capability::ALL.len()) - 1)
    }

    pub fn insert(&mut self, capability: Capability) {
        self.0 |= 1u64 << capability as u8;
    }

    #[must_use]
    pub const fn contains(self, capability: Capability) -> bool {
        self.0 & (1u64 << capability as u8) != 0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn is_subset_of(self, other: Self) -> bool {
        self.0 & !other.0 == 0
    }

    #[must_use]
    pub fn contains_all<I>(self, capabilities: I) -> bool
    where
        I: IntoIterator<Item = Capability>,
    {
        capabilities
            .into_iter()
            .all(|capability| self.contains(capability))
    }

    #[must_use]
    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    pub fn iter(self) -> impl Iterator<Item = Capability> {
        Capability::ALL
            .into_iter()
            .filter(move |capability| self.contains(*capability))
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<T: IntoIterator<Item = Capability>>(iterator: T) -> Self {
        let mut set = Self::empty();
        for capability in iterator {
            set.insert(capability);
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_keys_round_trip() {
        for capability in Capability::ALL {
            assert_eq!(Capability::parse(capability.as_str()), Some(capability));
        }
        assert_eq!(Capability::parse("nope"), None);
    }

    #[test]
    fn set_union_and_membership() {
        let mut set = CapabilitySet::empty();
        assert!(!set.contains(Capability::ContactsRead));
        assert!(set.is_empty());

        set.insert(Capability::ContactsRead);
        let other = [Capability::RostersWrite].into_iter().collect();
        let union = set.union(other);

        assert!(union.contains(Capability::ContactsRead));
        assert!(union.contains(Capability::RostersWrite));
        assert!(!union.contains(Capability::FinanceWrite));
        assert_eq!(union.len(), 2);
    }

    #[test]
    fn all_contains_every_capability() {
        let all = CapabilitySet::all();
        for capability in Capability::ALL {
            assert!(all.contains(capability));
        }
        assert_eq!(all.len(), Capability::ALL.len());
    }
}
