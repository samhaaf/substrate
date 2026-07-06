//! Logs (design §11 `logs`). Rich live tailing is DEFERRED in v1 (§13); this module
//! wraps the driver's `edge_logs` seam and a basic db-log query.

use substrate_types::Result;

use crate::driver::{Driver, Logs, Since};

/// Edge function logs (Management API on cloud, container on local, NI on sqlite).
pub async fn edge(driver: &dyn Driver, name: &str, since: &str) -> Result<Logs> {
    driver.edge_logs(name, Since(since.to_string())).await
}
