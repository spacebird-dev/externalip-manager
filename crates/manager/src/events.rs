use k8s_openapi::api::core::v1::ObjectReference;
use kube::{
    Client,
    runtime::events::{Event, EventType, Recorder, Reporter},
};
use tracing::warn;

#[derive(Clone)]
pub struct EventRecorder {
    recorder: Recorder,
}

impl EventRecorder {
    pub fn new(client: Client, controller: String) -> EventRecorder {
        EventRecorder {
            recorder: Recorder::new(
                client,
                Reporter {
                    controller,
                    instance: None,
                },
            ),
        }
    }

    pub async fn publish(
        &self,
        reason: String,
        action: String,
        type_: EventType,
        message: Option<String>,
        object_ref: &ObjectReference,
    ) {
        if let Err(e) = self
            .recorder
            .publish(
                &Event {
                    type_,
                    reason: reason.clone(),
                    note: message.clone(),
                    action: action.clone(),
                    secondary: None,
                },
                object_ref,
            )
            .await
        {
            warn!(msg = "failed to publish event for failing service", err = ?e, action, reason, note = message);
        }
    }
}
