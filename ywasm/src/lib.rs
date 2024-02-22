use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

mod array;
mod collection;
mod js;
mod transaction;

type Result<T> = std::result::Result<T, JsValue>;

#[wasm_bindgen]
#[repr(transparent)]
pub struct Observer(pub(crate) yrs::Subscription);
