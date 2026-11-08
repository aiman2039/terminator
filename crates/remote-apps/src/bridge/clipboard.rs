//! Clipboard text between the client and the remote apps, through wprsd's
//! data-device messages (selection only; no primary selection or drag and
//! drop yet).
//!
//! Copy in a remote app: wprsd says the selection changed; the bridge asks
//! for its text at once and sends it to the client as `ClipboardData`.
//! Paste into a remote app: the client sends `ClipboardOffer` with the text;
//! the bridge offers it as the selection and serves it when an app asks.

use wprs::serialization::Event;
use wprs::serialization::wayland::{
    DataDestinationEvent, DataDestinationRequest, DataEvent, DataRequest, DataSource,
    DataSourceEvent, DataSourceRequest, DataToTransfer, SourceMetadata,
};

use crate::protocol::{ClientMessage, Framed, HostMessage, TEXT_MIME_TYPES};

/// Largest clipboard text carried either way.
const MAX_TEXT: usize = 1024 * 1024;

#[derive(Default)]
pub(super) struct Clipboard {
    /// Text the client offered, served to apps that paste.
    local: Option<Vec<u8>>,
    /// The type asked from the remote selection, until its data comes.
    asked: Option<String>,
}

/// What a clipboard step sends: events to wprsd, frames to the client.
pub(super) type Outcome = (Vec<Event>, Vec<Framed<HostMessage>>);

impl Clipboard {
    pub(super) fn on_wprsd(&mut self, request: DataRequest) -> Outcome {
        match request {
            DataRequest::SourceRequest(DataSourceRequest::SetSelection(
                DataSource::Selection,
                metadata,
            )) => {
                let Some(mime) = TEXT_MIME_TYPES
                    .iter()
                    .find(|m| metadata.mime_types.iter().any(|offered| offered == *m))
                else {
                    return (Vec::new(), Vec::new());
                };
                self.asked = Some((*mime).to_owned());
                let ask = Event::Data(DataEvent::SourceEvent(
                    DataSourceEvent::MimeTypeSendRequestedByDestination(
                        DataSource::Selection,
                        (*mime).to_owned(),
                    ),
                ));
                (vec![ask], Vec::new())
            }
            DataRequest::TransferData(DataSource::Selection, DataToTransfer(bytes)) => {
                match self.asked.take() {
                    Some(mime_type) if bytes.len() <= MAX_TEXT => (
                        Vec::new(),
                        vec![Framed::with_payload(
                            HostMessage::ClipboardData { mime_type },
                            bytes,
                        )],
                    ),
                    _ => (Vec::new(), Vec::new()),
                }
            }
            DataRequest::DestinationRequest(DataDestinationRequest::RequestDataTransfer(
                DataSource::Selection,
                _,
            )) => match &self.local {
                Some(text) => (
                    vec![Event::Data(DataEvent::TransferData(
                        DataSource::Selection,
                        DataToTransfer(text.clone()),
                    ))],
                    Vec::new(),
                ),
                None => (Vec::new(), Vec::new()),
            },
            _ => (Vec::new(), Vec::new()),
        }
    }

    pub(super) fn on_client(&mut self, message: &ClientMessage, payload: Vec<u8>) -> Vec<Event> {
        let ClientMessage::ClipboardOffer { mime_types } = message else {
            return Vec::new();
        };
        if payload.len() > MAX_TEXT {
            return Vec::new();
        }
        self.local = Some(payload);
        vec![Event::Data(DataEvent::DestinationEvent(
            DataDestinationEvent::SelectionSet(
                DataSource::Selection,
                SourceMetadata {
                    mime_types: mime_types.clone(),
                    dnd_actions: 0,
                },
            ),
        ))]
    }
}
