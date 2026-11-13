use super::Ticket;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SaveTicket {
    request: u64,
    ticket: Ticket,
}

impl SaveTicket {
    fn observation(self) -> serde_json::Value {
        serde_json::json!({
            "request": self.request,
            "generation": self.ticket.generation,
            "serial": self.ticket.serial
        })
    }
}

#[derive(Default)]
pub(super) struct SaveFeedback {
    pub(super) requested: u64,
    submitted: Option<SaveTicket>,
    completed: Option<SaveTicket>,
}

impl SaveFeedback {
    pub(super) fn accept(&mut self) -> Option<u64> {
        let request = self.requested.checked_add(1)?;
        self.requested = request;
        Some(request)
    }

    pub(super) fn submit(&mut self, request: u64, ticket: Ticket) {
        self.submitted = Some(SaveTicket { request, ticket });
    }

    pub(super) fn complete(&mut self, ticket: Ticket) {
        if self
            .submitted
            .is_some_and(|submitted| submitted.ticket == ticket)
        {
            self.completed = self.submitted;
        }
    }

    pub(super) fn in_flight(&self) -> bool {
        self.submitted.is_some() && self.submitted != self.completed
    }

    pub(super) fn disconnected(&mut self) {
        self.submitted = None;
    }

    pub(super) fn replaced(&mut self) {
        self.submitted = None;
        self.completed = None;
    }

    pub(super) fn observation(&self, pending: bool) -> serde_json::Value {
        serde_json::json!({
            "requested": self.requested,
            "pending": pending,
            "submitted": self.submitted.map(SaveTicket::observation),
            "completed": self.completed.map(SaveTicket::observation)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceptance_is_distinct_from_dispatch_and_completion() {
        let mut feedback = SaveFeedback::default();
        assert_eq!(feedback.accept(), Some(1));
        assert!(!feedback.in_flight());
        assert_eq!(
            feedback.observation(true),
            serde_json::json!({
                "requested": 1, "pending": true, "submitted": null, "completed": null
            })
        );
        let ticket = Ticket {
            generation: 3,
            serial: 17,
        };
        feedback.submit(1, ticket);
        assert!(feedback.in_flight());
        feedback.complete(Ticket {
            generation: 2,
            serial: 17,
        });
        feedback.complete(Ticket {
            generation: 3,
            serial: 16,
        });
        assert!(feedback.in_flight());
        assert!(feedback.observation(true)["completed"].is_null());
        feedback.complete(ticket);
        assert!(!feedback.in_flight());
        assert_eq!(
            feedback.observation(false)["completed"],
            serde_json::json!({
                "request": 1, "generation": 3, "serial": 17
            })
        );
    }

    #[test]
    fn an_earlier_save_or_automatic_frame_cannot_complete_the_next_save() {
        let mut feedback = SaveFeedback::default();
        let first = Ticket {
            generation: 0,
            serial: 10,
        };
        let second = Ticket {
            generation: 0,
            serial: 12,
        };
        assert_eq!(feedback.accept(), Some(1));
        feedback.submit(1, first);
        assert_eq!(feedback.accept(), Some(2));
        feedback.complete(first);
        assert_eq!(feedback.observation(true)["completed"]["request"], 1);
        feedback.submit(2, second);
        feedback.complete(first);
        feedback.complete(Ticket {
            generation: 0,
            serial: 11,
        });
        assert!(feedback.in_flight());
        assert_eq!(feedback.observation(true)["completed"]["request"], 1);
        feedback.complete(second);
        assert!(!feedback.in_flight());
        assert_eq!(feedback.observation(false)["completed"]["request"], 2);
    }

    #[test]
    fn replacement_clears_old_owner_tickets_without_reusing_request_identities() {
        let mut feedback = SaveFeedback::default();
        let old = Ticket {
            generation: 0,
            serial: 10,
        };
        assert_eq!(feedback.accept(), Some(1));
        feedback.submit(1, old);
        feedback.complete(old);
        feedback.replaced();
        feedback.complete(old);
        assert!(!feedback.in_flight());
        assert!(feedback.observation(false)["submitted"].is_null());
        assert!(feedback.observation(false)["completed"].is_null());
        assert_eq!(feedback.accept(), Some(2));
    }

    #[test]
    fn exhausted_request_counter_does_not_wrap_or_mutate() {
        let mut feedback = SaveFeedback {
            requested: u64::MAX,
            ..Default::default()
        };
        let before = feedback.observation(false);
        assert_eq!(feedback.accept(), None);
        assert_eq!(feedback.observation(false), before);
    }

    #[test]
    fn disconnect_retires_the_unfinished_ticket_without_fabricating_completion() {
        let mut feedback = SaveFeedback::default();
        let completed = Ticket {
            generation: 0,
            serial: 10,
        };
        let interrupted = Ticket {
            generation: 0,
            serial: 12,
        };
        assert_eq!(feedback.accept(), Some(1));
        feedback.submit(1, completed);
        feedback.complete(completed);
        let before = feedback.observation(false)["completed"].clone();
        assert_eq!(feedback.accept(), Some(2));
        feedback.submit(2, interrupted);
        assert!(feedback.in_flight());
        feedback.disconnected();
        assert!(!feedback.in_flight());
        assert_eq!(feedback.requested, 2);
        assert!(feedback.observation(false)["submitted"].is_null());
        assert_eq!(feedback.observation(false)["completed"], before);
        feedback.complete(interrupted);
        assert_eq!(feedback.observation(false)["completed"], before);
        assert_eq!(feedback.accept(), Some(3));
    }
}
