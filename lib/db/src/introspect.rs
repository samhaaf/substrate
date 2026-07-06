//! Schema introspection SQL shared by the Postgres-backed drivers (design §10.3,
//! `inspect *`). The catalog queries live here so cloud + local render identically.

use crate::driver::IntrospectQuery;

/// The Postgres catalog SQL for an [`IntrospectQuery`].
pub fn introspect_sql(q: &IntrospectQuery) -> String {
    match q {
        IntrospectQuery::Tables => {
            "SELECT table_schema, table_name FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY 1,2"
                .to_string()
        }
        IntrospectQuery::Describe { table } => format!(
            "SELECT column_name, data_type, is_nullable FROM information_schema.columns \
             WHERE table_name = '{table}' ORDER BY ordinal_position"
        ),
        IntrospectQuery::Functions => {
            "SELECT n.nspname AS schema, p.proname AS name \
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname NOT IN ('pg_catalog','information_schema') ORDER BY 1,2"
                .to_string()
        }
        IntrospectQuery::Triggers => {
            "SELECT event_object_table AS table, trigger_name FROM information_schema.triggers \
             ORDER BY 1,2"
                .to_string()
        }
        IntrospectQuery::Policies => {
            "SELECT schemaname, tablename, policyname FROM pg_policies ORDER BY 1,2,3".to_string()
        }
        IntrospectQuery::Handlers => {
            "SELECT h.name, ha.env, hv.version, hv.kind, hv.invocation \
             FROM ops.handlers h \
             LEFT JOIN ops.handler_active ha ON ha.handler = h.name \
             LEFT JOIN ops.handler_versions hv ON hv.id = ha.version_id ORDER BY 1,2"
                .to_string()
        }
        IntrospectQuery::Size => {
            "SELECT relname AS table, pg_size_pretty(pg_total_relation_size(c.oid)) AS size \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind='r' AND n.nspname NOT IN ('pg_catalog','information_schema') \
             ORDER BY pg_total_relation_size(c.oid) DESC"
                .to_string()
        }
    }
}
