use crate::ID;

/**
 * General Notes:
 * Each type internally is a linked list
 *
 * Does ArrayCursor work for Text
 * Can it be made to work?
 *
 *
 *
 */
pub struct RelativePosition {
    type_id: ID,
    type_name: Option<String>,
    // item seems to do a LOT of heavy lifting in terms of logical clock and ordering
    item: ID,
    // for left/right association. Does not seem to be implemented at all in yrs yet
    association: usize,
}

pub struct AbsolutePosition {}

//may need to check for BranchPtr
trait HasBranchPtr {}

pub fn create_relative_position_from_type_index<T>(shared_type: T) -> Option<RelativePosition> {
    unimplemented!()
}

// pub fn create_relative_positon_from_json() -> Option<RelativePosition> {}

// pub fn create_absolute_positon_from_relative_position() -> Option<AbsolutePosition> {}

// pub fn compare_relative_positions() {}

#[cfg(test)]
mod test {
    use crate::block::ItemContent;
    use crate::cursor::{Assoc, MoveIter};
    use crate::{Doc, ID};
}
