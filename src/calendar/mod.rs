mod client;
mod types;

pub use client::CalendarWebhookClient;
pub use types::{
    CalendarWebhookRequest, CalendarWebhookResponse, ConfirmationRequest, EventCandidate,
};
