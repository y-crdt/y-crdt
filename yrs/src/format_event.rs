//! Bounded, read-only observation of one format-only update against its exact prestate.
//! This exposes CRDT identities, not application authorship or authorization.
use crate::block::ItemContent;
use crate::branch::BranchID;
use crate::types::TypeRef;
use crate::update::BlockCarrier;
use crate::updates::decoder::Decode;
use crate::{Any, Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, Update, ID};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatTarget {
    pub branch: BranchID,
    pub item: ID,
}
#[derive(Debug, Clone, PartialEq)]
pub struct FormatState {
    pub marker: Option<ID>,
    pub value: Any,
}
#[derive(Debug, Clone, PartialEq)]
pub struct FormatChange {
    pub target: FormatTarget,
    pub before: FormatState,
    pub after: FormatState,
}
#[derive(Debug, Clone, PartialEq)]
pub struct FormatEvent {
    pub inserted: Vec<ID>,
    pub deleted: Vec<ID>,
    pub changes: Vec<FormatChange>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FormatEventError {
    #[error("format event limit exceeded")]
    Limit,
    #[error("format event requires settled UTF-16 prestate")]
    Prestate,
    #[error("invalid or incomplete update")]
    Update,
    #[error("unavailable or duplicate text target")]
    Target,
    #[error("update contains unprovable or non-format content")]
    Unprovable,
    #[error("format event has no observable target change")]
    Ineffective,
}

impl FormatEvent {
    /// Inspect exact v1 bytes without modifying `txn` or its document. Values and real
    /// marker IDs are retained, including IDs deleted by a removal. This is not a
    /// validator for an application's claimed selection: applications must additionally
    /// validate whole-document semantic changes and authenticate the event identity.
    /// Merged/GC events whose marker content is lost are refused. Limits: 131072 targets,
    /// 4096 markers per side, and 8 MiB update/checkpoint. The checkpoint limit is
    /// checked after encoding; encoding and scratch replay cost scale with document
    /// size and are not charged to `max_steps`. That budget (at most one million)
    /// bounds explicit target/ancestor and event traversal only. Callers must also
    /// bound the input document before invoking this API.
    pub fn inspect<T: ReadTxn>(txn: &T, bytes: &[u8], attribute: &str, targets: &[FormatTarget], max_steps: usize) -> Result<Self, FormatEventError> {
        if bytes.is_empty() || bytes.len() > 8*1024*1024 || attribute.is_empty() || attribute.len()>256 || targets.is_empty() || targets.len()>131072 || max_steps==0 || max_steps>1_000_000 {
            return Err(FormatEventError::Limit);
        }
        if txn.store().offset_kind != OffsetKind::Utf16 || txn.store().pending_update().is_some() || txn.store().pending_ds().is_some() { return Err(FormatEventError::Prestate); }
        let unique: HashSet<_> = targets.iter().collect();
        if unique.len()!=targets.len() { return Err(FormatEventError::Target); }
        let mut budget=max_steps;
        let before=observe(txn,attribute,targets,&mut budget)?;
        let update=Update::decode_v1(bytes).map_err(|_|FormatEventError::Update)?;
        let mut inserted=Vec::new(); let mut new=HashSet::new();
        for block in update.blocks.blocks() {
            step(&mut budget)?;
            match block {
                BlockCarrier::Item(item) => match &item.content {
                    ItemContent::Format(key,_) if key.as_ref()==attribute && item.len==1 && txn.store().blocks.get_block(&item.id).is_none() => {
                        if inserted.len()==4096 { return Err(FormatEventError::Limit); }
                        inserted.push(item.id); new.insert(item.id);
                    }
                    _=>return Err(FormatEventError::Unprovable),
                },
                _=>return Err(FormatEventError::Unprovable),
            }
        }
        let mut deleted=Vec::new();
        for (client,ranges) in update.delete_set.iter() { for range in ranges.iter() {
            if range.end-range.start>4096 || deleted.len()+(range.end-range.start) as usize>4096 { return Err(FormatEventError::Limit); }
            for clock in range.clone() {
                step(&mut budget)?;
                let id=ID::new(*client,clock);
                if !new.contains(&id) {
                    let item=txn.store().blocks.get_item(&id).ok_or(FormatEventError::Unprovable)?;
                    if item.is_deleted() || item.len!=1 || !matches!(&item.content,ItemContent::Format(key,_) if key.as_ref()==attribute) { return Err(FormatEventError::Unprovable); }
                }
                deleted.push(id);
            }
        }}
        if inserted.is_empty() && deleted.is_empty() { return Err(FormatEventError::Ineffective); }
        let checkpoint=txn.encode_state_as_update_v1(&StateVector::default());
        if checkpoint.len()>8*1024*1024 { return Err(FormatEventError::Limit); }
        let scratch=Doc::with_options(Options{offset_kind:OffsetKind::Utf16,skip_gc:true,..Options::default()});
        {
            let mut t=scratch.transact_mut();
            t.apply_update(Update::decode_v1(&checkpoint).map_err(|_|FormatEventError::Update)?).map_err(|_|FormatEventError::Update)?;
            t.apply_update(update).map_err(|_|FormatEventError::Update)?;
        }
        let t=scratch.transact();
        if t.store().pending_update().is_some() || t.store().pending_ds().is_some() { return Err(FormatEventError::Update); }
        let after=observe(&t,attribute,targets,&mut budget)?;
        let changes=targets.iter().zip(before).zip(after).filter_map(|((target,before),after)|{
            if before==after {None} else {Some(FormatChange{target:target.clone(),before,after})}
        }).collect::<Vec<_>>();
        if changes.is_empty() { return Err(FormatEventError::Ineffective); }
        inserted.sort();deleted.sort();
        Ok(Self{inserted,deleted,changes})
    }
}
fn step(budget:&mut usize)->Result<(),FormatEventError>{
    *budget=budget.checked_sub(1).ok_or(FormatEventError::Limit)?;Ok(())
}
fn observe<T:ReadTxn>(txn:&T,attribute:&str,targets:&[FormatTarget],budget:&mut usize)->Result<Vec<FormatState>,FormatEventError>{
    let mut branches:HashMap<BranchID,HashSet<ID>>=HashMap::new();
    for target in targets { branches.entry(target.branch.clone()).or_default().insert(target.item); }
    let mut found=HashMap::new();
    for (id,wanted) in branches {
        let branch=id.get_branch(txn).ok_or(FormatEventError::Target)?;
        if !matches!(branch.type_ref(),TypeRef::Text|TypeRef::XmlText|TypeRef::Undefined)  { return Err(FormatEventError::Target); }
        // A live child is not proof that its containing XML subtree is visible.
        let mut ancestor=branch.item;
        while let Some(item)=ancestor {
            step(budget)?;
            if item.is_deleted() { return Err(FormatEventError::Target); }
            let parent=item.parent.as_branch().ok_or(FormatEventError::Target)?;
            ancestor=parent.item;
        }
        let mut marker=None;let mut value=Any::Null;let mut next=branch.start;
        while let Some(item)=next {
            step(budget)?;next=item.right;
            if item.is_deleted(){continue;}
            match &item.content {
                ItemContent::Format(key,v) if key.as_ref()==attribute=>{marker=Some(item.id);value=(**v).clone();},
                ItemContent::String(_)=>{
                    for clock in item.id.clock..item.id.clock+item.len {
                        step(budget)?;let unit=ID::new(item.id.client,clock);
                        if wanted.contains(&unit){found.insert((id.clone(),unit),FormatState{marker,value:value.clone()});}
                    }
                },
                ItemContent::Format(_,_)=>{},
                _=>return Err(FormatEventError::Unprovable),
            }
        }
    }
    targets.iter().map(|target|found.get(&(target.branch.clone(),target.item)).cloned().ok_or(FormatEventError::Target)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Assoc, IndexedSequence, Text};
    use std::sync::Arc;
    fn setup() -> (Doc, crate::TextRef, Vec<FormatTarget>) {
        let doc=Doc::with_options(Options{offset_kind:OffsetKind::Utf16,..Options::default()});
        let text=doc.get_or_insert_text("text");
        text.insert(&mut doc.transact_mut(),0,"Hello");
        let targets={let txn=doc.transact();(0..5).map(|i|FormatTarget{branch:BranchID::Root(Arc::from("text")),item:*text.sticky_index(&txn,i,Assoc::After).unwrap().id().unwrap()}).collect()};
        (doc,text,targets)
    }
    #[test]
    fn additions_removals_and_aba_retain_real_marker_ids() {
        let (doc,text,targets)=setup();
        let receiver=Doc::with_options(Options{offset_kind:OffsetKind::Utf16,..Options::default()});
        receiver.transact_mut().apply_update(Update::decode_v1(&doc.transact().encode_state_as_update_v1(&StateVector::default())).unwrap()).unwrap();
        let mut events=Vec::new();
        for value in [Any::Bool(true),Any::Null,Any::Bool(true),Any::Null] {
            let bytes={let mut txn=doc.transact_mut();text.format(&mut txn,0,5,[(Arc::from("bold"),value)].into());txn.encode_update_v1()};
            let before=receiver.transact().encode_state_as_update_v1(&StateVector::default());
            let event=FormatEvent::inspect(&receiver.transact(),&bytes,"bold",&targets,10000).unwrap();
            assert_eq!(receiver.transact().encode_state_as_update_v1(&StateVector::default()),before);
            events.push(event);
            receiver.transact_mut().apply_update(Update::decode_v1(&bytes).unwrap()).unwrap();
        }
        assert_ne!(events[1].deleted,events[3].deleted);
        assert!(events[1].changes.iter().all(|c|c.after.value==Any::Null));
        assert!(events[3].changes.iter().all(|c|c.after.value==Any::Null));
    }
    #[test]
    fn malformed_bounds_duplicate_targets_and_nonformat_updates_refuse() {
        let (doc,text,targets)=setup();
        assert_eq!(FormatEvent::inspect(&doc.transact(),&[255],"bold",&targets,1000),Err(FormatEventError::Update));
        assert_eq!(FormatEvent::inspect(&doc.transact(),&[0,0],"bold",&targets,0),Err(FormatEventError::Limit));
        assert_eq!(FormatEvent::inspect(&doc.transact(),&[0,0],"bold",&[targets[0].clone(),targets[0].clone()],1000),Err(FormatEventError::Target));
        let before=Doc::with_options(Options{offset_kind:OffsetKind::Utf16,..Options::default()});
        before.transact_mut().apply_update(Update::decode_v1(&doc.transact().encode_state_as_update_v1(&StateVector::default())).unwrap()).unwrap();
        let bytes={let mut txn=doc.transact_mut();text.insert(&mut txn,0,"X");txn.encode_update_v1()};
        assert_eq!(FormatEvent::inspect(&before.transact(),&bytes,"bold",&targets,1000),Err(FormatEventError::Unprovable));
    }
    #[test]
    fn deleted_xml_ancestor_refuses_target() {
        use crate::{XmlFragment, XmlElementPrelim, XmlTextPrelim, SharedRef};
        let doc=Doc::with_options(Options{offset_kind:OffsetKind::Utf16,skip_gc:true,..Options::default()});
        let root=doc.get_or_insert_xml_fragment("xml");
        let text={let mut txn=doc.transact_mut();let p=root.push_back(&mut txn,XmlElementPrelim::empty("p"));p.push_back(&mut txn,XmlTextPrelim::new("Hello"))};
        let target={let txn=doc.transact();FormatTarget{branch:text.hook().id().clone(),item:*text.sticky_index(&txn,0,Assoc::After).unwrap().id().unwrap()}};
        root.remove_range(&mut doc.transact_mut(),0,1);
        assert_eq!(FormatEvent::inspect(&doc.transact(),&[0,0],"bold",&[target],1000),Err(FormatEventError::Target));
    }

    #[test]
    fn actual_yjs_format_event_parity() {
        use serde_json::Value;
        fn id(v:&Value)->ID { ID::new(v["client"].as_u64().unwrap(),v["clock"].as_u64().unwrap() as u32) }
        fn bytes(v:&Value)->Vec<u8> { v.as_array().unwrap().iter().map(|v|v.as_u64().unwrap() as u8).collect() }
        let fixture:Value=serde_json::from_str(include_str!("fixtures/format_event_js_v1.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let doc=Doc::with_options(Options{offset_kind:OffsetKind::Utf16,..Options::default()});
            doc.transact_mut().apply_update(Update::decode_v1(&bytes(&case["before"])).unwrap()).unwrap();
            let expected=&case["event"];
            let targets:Vec<_>=expected["changes"].as_array().unwrap().iter().map(|c|FormatTarget{branch:BranchID::Nested(id(&c["unit"]["type"])),item:id(&c["unit"]["item"])}).collect();
            let event=FormatEvent::inspect(&doc.transact(),&bytes(&case["update"]),"bold",&targets,10000).unwrap();
            for (actual,key) in [(&event.inserted,"inserted_markers"),(&event.deleted,"deleted_markers")] {
                let mut ids:Vec<_>=expected[key].as_array().unwrap().iter().map(id).collect();ids.sort();assert_eq!(*actual,ids);
            }
            assert_eq!(event.changes.len(),targets.len());
            for (actual,expected) in event.changes.iter().zip(expected["changes"].as_array().unwrap()) {
                for (state,key) in [(&actual.before,"before"),(&actual.after,"after")] {
                    let marker=&expected[key]["marker"];
                    assert_eq!(state.marker,if marker.is_null(){None}else{Some(id(marker))});
                    if expected[key]["enabled"].as_bool().unwrap() { assert_eq!(state.value,Any::Map(Arc::new(Default::default()))); }
                    else { assert_eq!(state.value,Any::Null); }
                }
            }
        }
    }

}
