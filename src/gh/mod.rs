pub mod detect;
pub mod pr;

pub use detect::{Availability, detect};
pub use pr::{fetch_prs, Pr, PrState};
