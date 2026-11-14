//! 编译期冒烟：宏能展开；不加载真实 Node。

use wae::{CallContext, JsValue, Result, function, object, register_module};

#[function]
fn version(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string("0.0.0-test")
}

#[function(js_name = "helloWorld")]
fn hello_world(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string("hi")
}

/// 字段结构的 JS 形状：camelCase 映射、可选字段、显式改名。
#[object]
#[derive(Debug, Clone, PartialEq)]
struct RenderOptions {
    out: String,
    sample_rate: Option<u32>,
    #[js_name = "presetName"]
    preset: Option<String>,
}

/// 类型化导出：参数与返回值都参与声明生成。
#[function(js_name = "render")]
fn render(options: RenderOptions, count: Option<u32>) -> Result<String> {
    Ok(format!("{}:{}:{}", options.out, options.sample_rate.unwrap_or(0), count.unwrap_or(0)))
}

register_module![version, hello_world, render];

#[test]
fn export_const_names() {
    assert_eq!(WAE_NAPI_EXPORT_VERSION.0, "version");
    assert_eq!(WAE_NAPI_EXPORT_HELLO_WORLD.0, "helloWorld");
    assert_eq!(WAE_NAPI_EXPORT_RENDER.0, "render");
}

#[test]
fn typed_metadata_describes_params_returns_and_objects() {
    let json = wae::metadata_json(&[wae_meta_render]);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("metadata json");
    let exports = parsed["exports"].as_array().expect("exports");
    assert_eq!(exports.len(), 1);
    let export = &exports[0];
    assert_eq!(export["js_name"], "render");
    assert_eq!(export["returns"], "string");
    assert_eq!(export["untyped"], false);

    let params = export["params"].as_array().expect("params");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0]["name"], "options");
    assert_eq!(params[0]["ts"], "RenderOptions");
    assert_eq!(params[0]["optional"], false);
    assert_eq!(params[1]["name"], "count");
    assert_eq!(params[1]["ts"], "number");
    assert_eq!(params[1]["optional"], true);

    let types = parsed["types"].as_array().expect("types");
    assert_eq!(types.len(), 1);
    let declaration = types[0]["text"].as_str().expect("declaration");
    assert!(declaration.contains("export interface RenderOptions {"), "{declaration}");
    assert!(declaration.contains("out: string"), "{declaration}");
    assert!(declaration.contains("sampleRate?: number"), "{declaration}");
    assert!(declaration.contains("presetName?: string"), "{declaration}");
}

#[test]
fn legacy_exports_stay_untyped_in_metadata() {
    let json = wae::metadata_json(&[wae_meta_version, wae_meta_hello_world]);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("metadata json");
    for export in parsed["exports"].as_array().expect("exports") {
        assert_eq!(export["untyped"], true);
        assert_eq!(export["returns"], "unknown");
        assert_eq!(export["params"].as_array().map(Vec::len), Some(0));
    }
    assert_eq!(parsed["types"].as_array().map(Vec::len), Some(0));
}

#[test]
fn metadata_is_sorted_by_export_name() {
    let json = wae::metadata_json(&[wae_meta_version, wae_meta_render, wae_meta_hello_world]);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("metadata json");
    let names: Vec<&str> = parsed["exports"].as_array().expect("exports").iter().filter_map(|export| export["js_name"].as_str()).collect();
    assert_eq!(names, ["helloWorld", "render", "version"]);
}
