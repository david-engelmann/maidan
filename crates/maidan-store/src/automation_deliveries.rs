//! Automation HTTP delivery queue (slash commands + FSM hooks).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationDeliveryFilter {
    Pending,
    DeadLetter,
    Delivered,
}
