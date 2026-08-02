//! APNs device token storage
//!
//! CRUD operations for Apple Push Notification service device registrations.

mod model;
mod store;

pub use model::{ApnsDevice, ApnsDeviceRegistration};
pub use store::ApnsDeviceStore;
