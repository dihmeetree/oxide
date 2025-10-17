/// Web dashboard for cluster management
mod cache;
pub mod insights;
mod routes;
mod server;
mod templates;

pub use server::DashboardServer;
