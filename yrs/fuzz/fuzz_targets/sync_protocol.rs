#![no_main]
//! Processes a stream of y-sync protocol messages (sync steps, updates, awareness, auth)
//! the way a server or a peer would.

use libfuzzer_sys::fuzz_target;
use yrs::encoding::read::Cursor;
use yrs::sync::awareness::AwarenessUpdate;
use yrs::sync::{Awareness, Message, MessageReader, SyncMessage};
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, Transact, Update};

fuzz_target!(|data: &[u8]| {
    let doc = yrs_fuzz::load_doc(&yrs_fuzz::seeds().state_v1, false);
    let mut awareness = Awareness::new(doc);
    let mut decoder = DecoderV1::new(Cursor::new(data));
    for msg in MessageReader::new(&mut decoder).take(32) {
        match msg {
            Ok(Message::Sync(SyncMessage::SyncStep1(sv))) => {
                let _ = awareness.doc().transact().encode_state_as_update_v1(&sv);
            }
            Ok(Message::Sync(SyncMessage::SyncStep2(update)))
            | Ok(Message::Sync(SyncMessage::Update(update))) => {
                if let Ok(update) = Update::decode_v1(&update) {
                    let _ = awareness.doc().transact_mut().apply_update(update);
                }
            }
            Ok(Message::Awareness(update)) => {
                let _ = awareness.apply_update(update);
                if let Ok(update) = awareness.update() {
                    let _ = update.encode_v1();
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    yrs_fuzz::exercise(awareness.doc());

    if let Ok(update) = AwarenessUpdate::decode_v1(data) {
        let mut awareness = Awareness::new(yrs_fuzz::doc_with_client(100, false));
        let _ = awareness.apply_update(update);
        let _ = awareness.update();
    }
});
