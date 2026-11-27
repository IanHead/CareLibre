-- External counterparties and internal branches share a tenant. Names should be
-- unique within that tenant, independently of the case used, so two records
-- cannot drift apart in the relationship picker.
CREATE UNIQUE INDEX organisations_tenant_child_name_key
    ON organisations (tenant_id, lower(name))
    WHERE tenant_id <> id;
