//! 编译期冒烟：宏能展开；不加载真实 Node。

use wae::{CallContext, JsValue, Result, function, register_module};

#[function]
fn version(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string("0.0.0-test")
}

#[function(js_name = "helloWorld")]
fn hello_world(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string("hi")
}

register_module![version, hello_world];

#[test]
fn export_const_names() {
    assert_eq!(WAE_NAPI_EXPORT_VERSION.0, "version");
    assert_eq!(WAE_NAPI_EXPORT_HELLO_WORLD.0, "helloWorld");
}
