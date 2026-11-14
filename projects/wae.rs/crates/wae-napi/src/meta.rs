//! 导出面的类型元数据。
//!
//! `#[function]` / `#[object]` 在编译期生成这里的常量，模块初始化时拼成 JSON，
//! 由 `@wae/wae` 的 `types` 构建步渲染成单份 `src/native.ts`。
//! 元数据来自 Rust 源码而不是某一块二进制，因此与构建目标无关。

/// 一个对象的 TypeScript 接口声明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDecl {
    /// 接口名（等于 Rust 类型名）。
    pub name: String,
    /// 完整 `export interface ...` 文本。
    pub text: String,
}

/// Rust 类型对应的 TypeScript 类型。
pub trait TsType {
    /// TypeScript 类型表达式，例如 `string` 或 `Array<Foo>`。
    fn ts_type() -> String;

    /// 追加该类型依赖的接口声明；基础类型没有。
    fn declarations(_out: &mut Vec<TypeDecl>) {}
}

macro_rules! simple_ts_type {
    ($($ty:ty => $ts:expr),* $(,)?) => {
        $(
            impl TsType for $ty {
                fn ts_type() -> String {
                    $ts.to_string()
                }
            }
        )*
    };
}

simple_ts_type!(
    () => "undefined",
    bool => "boolean",
    String => "string",
    &str => "string",
    f32 => "number",
    f64 => "number",
    i8 => "number",
    i16 => "number",
    i32 => "number",
    i64 => "number",
    isize => "number",
    u8 => "number",
    u16 => "number",
    u32 => "number",
    u64 => "number",
    usize => "number",
    serde_json::Value => "unknown",
    crate::JsValue => "unknown",
);

impl<T: TsType> TsType for Option<T> {
    fn ts_type() -> String {
        format!("{} | undefined", T::ts_type())
    }

    fn declarations(out: &mut Vec<TypeDecl>) {
        T::declarations(out);
    }
}

impl<T: TsType> TsType for Vec<T> {
    fn ts_type() -> String {
        format!("Array<{}>", T::ts_type())
    }

    fn declarations(out: &mut Vec<TypeDecl>) {
        T::declarations(out);
    }
}

impl<T: TsType> TsType for std::collections::HashMap<String, T> {
    fn ts_type() -> String {
        format!("Record<string, {}>", T::ts_type())
    }

    fn declarations(out: &mut Vec<TypeDecl>) {
        T::declarations(out);
    }
}

impl<T> TsType for crate::Json<T> {
    fn ts_type() -> String {
        "unknown".to_string()
    }
}

/// 一个导出参数的 TypeScript 形状。
#[derive(Debug, Clone)]
pub struct ParamMeta {
    /// 参数名，取自 Rust 形参标识符。
    pub name: &'static str,
    /// TypeScript 类型表达式。
    pub ts: fn() -> String,
    /// `Option<T>` 形参在 JS 侧可省略。
    pub optional: bool,
}

/// 一个导出函数的 TypeScript 形状。
#[derive(Debug, Clone)]
pub struct FunctionMeta {
    /// 模块上的导出名。
    pub js_name: &'static str,
    /// 位置参数。
    pub params: &'static [ParamMeta],
    /// 返回值类型。
    pub returns: fn() -> String,
    /// 旧形态（`fn(CallContext) -> Result<JsValue>`）：参数面不可静态描述。
    pub untyped: bool,
    /// `#[function(async)]`：JS 侧返回 `Promise<returns>`。
    pub is_async: bool,
    /// 追加该函数用到的接口声明。
    pub declarations: fn(&mut Vec<TypeDecl>),
}

/// 把一组函数元数据渲染成 JSON 文本。
///
/// 导出按 `js_name` 排序、接口按名字去重，保证同一份源码在任何目标上生成同一份文本。
pub fn metadata_json(functions: &[fn() -> FunctionMeta]) -> String {
    let mut exports: Vec<serde_json::Value> = Vec::with_capacity(functions.len());
    let mut types: Vec<TypeDecl> = Vec::new();
    for factory in functions {
        let meta = factory();
        (meta.declarations)(&mut types);
        if meta.untyped {
            exports.push(serde_json::json!({
                "js_name": meta.js_name,
                "params": [],
                "returns": "unknown",
                "untyped": true,
                "async": meta.is_async,
            }));
            continue;
        }
        let params: Vec<serde_json::Value> = meta
            .params
            .iter()
            .map(|param| {
                serde_json::json!({
                    "name": param.name,
                    "ts": (param.ts)(),
                    "optional": param.optional,
                })
            })
            .collect();
        exports.push(serde_json::json!({
            "js_name": meta.js_name,
            "params": params,
            "returns": (meta.returns)(),
            "untyped": false,
            "async": meta.is_async,
        }));
    }
    exports.sort_by(|left, right| {
        let left = left.get("js_name").and_then(serde_json::Value::as_str).unwrap_or_default();
        let right = right.get("js_name").and_then(serde_json::Value::as_str).unwrap_or_default();
        left.cmp(right)
    });
    types.sort_by(|left, right| left.name.cmp(&right.name));
    types.dedup_by(|left, right| left.name == right.name);
    serde_json::json!({
        "generator": "wae",
        "wae_version": env!("CARGO_PKG_VERSION"),
        "exports": exports,
        "types": types.iter().map(|decl| serde_json::json!({ "name": decl.name, "text": decl.text })).collect::<Vec<_>>(),
    })
    .to_string()
}
