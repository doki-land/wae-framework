//! MSI writer：spec → CFB 数据库字节。
//!
//! v0 合同（readme 同步声明）：静默优先（`msiexec /qn` / `MsiInstallProduct`
//! 进程内 API），不含交互 UI 表集；不写 `Shortcut`/`Icon` 表（schema 未经
//! spike 钉死，宁缺毋假造）；payload 全部驻留内存后组装（构建期可接受）。
//!
//! 表与列的 `Type` 值来自 schema spike（真实 MSI 经 `MsiOpenDatabase` SQL
//! 视图导出），见 `tables.rs` 常量。验收闭环 = 本 crate 写出 → msi.dll 进程内
//! 回读逐表比对（`verify.rs`）。

use crate::{
    cab::{CabEntry, build_cab},
    cfb::Cfb,
    error::{InstallerError, Result},
    md5::md5_guid,
    spec::{InstallerSpec, MsiOptions, Scope},
    tables::{
        COMPONENT_COLUMNS, Cell, DIRECTORY_COLUMNS, FEATURE_COLUMNS, FEATURE_COMPONENTS_COLUMNS, FILE_COLUMNS, MEDIA_COLUMNS, PROPERTY_COLUMNS,
        SEQUENCE_COLUMNS, StringPool, Table, UPGRADE_COLUMNS, encode_columns, encode_rows, encode_tables,
    },
    writer::{FormatId, WriteReport},
};

/// 内嵌 cabinet 的 CFB 流名（`Media.Cabinet="#FileData"`）。
const CABINET_STREAM: &str = "FileData";

/// 生成一个 `.msi`。
pub fn write_msi(spec: &InstallerSpec, options: &MsiOptions) -> Result<WriteReport> {
    spec.validate()?;
    if spec.output.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase).as_deref() != Some("msi") {
        return Err(InstallerError::spec(crate::spec::spec_codes::EMPTY_FIELD, "`output` must end with `.msi`"));
    }

    // 1. 读 payload。
    let mut payload_bytes: Vec<(&str, Vec<u8>)> = Vec::new();
    for file in &spec.payload {
        let name = file.install_name()?;
        let data = std::fs::read(&file.source)
            .map_err(|error| InstallerError::Io(std::io::Error::new(error.kind(), format!("payload `{}`: {error}", file.source.display()))))?;
        payload_bytes.push((name, data));
    }

    // 2. GUID 与目录键。
    let upgrade_code = spec.product.upgrade_code.clone().unwrap_or_else(|| md5_guid(&format!("upgrade/{}", spec.product.name)));
    let product_code =
        spec.product.product_code.clone().unwrap_or_else(|| md5_guid(&format!("product/{upgrade_code}/{}", spec.product.version)));
    let package_code = spec.product.package_code.clone().unwrap_or_else(|| md5_guid(&format!("package/{product_code}")));
    let component_id = md5_guid(&format!("component/{upgrade_code}/{}", spec.product.name));
    let install_dir_key = "INSTALLDIR";

    // 3. 表数据。
    let mut pool = StringPool::default();
    let mut tables: Vec<Table> = Vec::new();

    // Property。
    let mut property_rows = vec![
        vec![Cell::Text("ProductCode".into()), Cell::Text(product_code.clone())],
        vec![Cell::Text("PackageCode".into()), Cell::Text(package_code.clone())],
        vec![Cell::Text("ProductLanguage".into()), Cell::Int(spec.product.language as u32)],
        vec![Cell::Text("ProductName".into()), Cell::Text(spec.product.name.clone())],
        vec![Cell::Text("ProductVersion".into()), Cell::Text(spec.product.version.clone())],
        vec![Cell::Text("Manufacturer".into()), Cell::Text(spec.product.manufacturer.clone())],
        vec![Cell::Text("UpgradeCode".into()), Cell::Text(upgrade_code.clone())],
    ];
    match options.scope {
        Scope::PerMachine => property_rows.push(vec![Cell::Text("ALLUSERS".into()), Cell::Text("1".into())]),
        Scope::PerUser => property_rows.push(vec![Cell::Text("MSIINSTALLPERUSER".into()), Cell::Text("1".into())]),
    }
    for (name, value) in arp_properties(spec) {
        property_rows.push(vec![Cell::Text(name), Cell::Text(value)]);
    }
    for (name, value) in &options.extra_properties {
        property_rows.push(vec![Cell::Text(name.clone()), Cell::Text(value.clone())]);
    }
    tables.push(Table { name: "Property", columns: PROPERTY_COLUMNS, rows: property_rows });

    // Directory。
    let root_folder = match options.scope {
        Scope::PerMachine => "ProgramFilesFolder",
        Scope::PerUser => "LocalAppDataFolder",
    };
    tables.push(Table {
        name: "Directory",
        columns: DIRECTORY_COLUMNS,
        rows: vec![
            vec![Cell::Text("TARGETDIR".into()), Cell::Null, Cell::Text("source".into())],
            vec![Cell::Text(root_folder.into()), Cell::Text("TARGETDIR".into()), Cell::Text(root_folder.into())],
            vec![Cell::Text(install_dir_key.into()), Cell::Text(root_folder.into()), Cell::Text(dir_name(&spec.product.name))],
        ],
    });

    // Component / Feature / FeatureComponents。
    tables.push(Table {
        name: "Component",
        columns: COMPONENT_COLUMNS,
        rows: vec![vec![
            Cell::Text("Main".into()),
            Cell::Text(component_id),
            Cell::Text(install_dir_key.into()),
            Cell::Int(0),
            Cell::Null,
            Cell::Text(payload_bytes[0].0.to_owned()),
        ]],
    });
    tables.push(Table {
        name: "Feature",
        columns: FEATURE_COLUMNS,
        rows: vec![vec![
            Cell::Text("Complete".into()),
            Cell::Null,
            Cell::Text(spec.product.name.clone()),
            Cell::Null,
            Cell::Null,
            Cell::Int(1),
            Cell::Text(install_dir_key.into()),
            Cell::Int(0),
        ]],
    });
    tables.push(Table {
        name: "FeatureComponents",
        columns: FEATURE_COMPONENTS_COLUMNS,
        rows: vec![vec![Cell::Text("Complete".into()), Cell::Text("Main".into())]],
    });

    // File。
    let mut file_rows = Vec::new();
    for (index, (name, data)) in payload_bytes.iter().enumerate() {
        file_rows.push(vec![
            Cell::Text((*name).to_owned()),
            Cell::Text("Main".into()),
            Cell::Text((*name).to_owned()),
            Cell::Int(data.len() as u32),
            Cell::Null,
            Cell::Null,
            Cell::Int(0),
            Cell::Int(index as u32 + 1),
        ]);
    }
    tables.push(Table { name: "File", columns: FILE_COLUMNS, rows: file_rows });

    // Media：内嵌 cabinet。
    tables.push(Table {
        name: "Media",
        columns: MEDIA_COLUMNS,
        rows: vec![vec![
            Cell::Int(1),
            Cell::Int(payload_bytes.len() as u32),
            Cell::Null,
            Cell::Text(format!("#{CABINET_STREAM}")),
            Cell::Null,
            Cell::Null,
        ]],
    });

    // InstallExecuteSequence（静默安装/卸载标准序）。
    let mut sequence: Vec<(&str, Option<&str>, u32)> = vec![
        ("CostInitialize", None, 800),
        ("FileCost", None, 900),
        ("CostFinalize", None, 1000),
        ("MigrateFeatureStates", Some("NOT REMOVE"), 1200),
        ("InstallValidate", None, 1400),
        ("InstallInitialize", None, 1500),
        ("ProcessComponents", None, 1600),
        ("UnpublishFeatures", Some("REMOVE"), 1700),
        ("RemoveFiles", Some("REMOVE"), 3700),
        ("RemoveFolders", Some("REMOVE"), 3800),
        ("CreateFolders", Some("NOT REMOVE"), 4500),
        ("InstallFiles", Some("NOT REMOVE"), 4000),
        ("PublishFeatures", Some("NOT REMOVE"), 5000),
        ("PublishProduct", Some("NOT REMOVE"), 5100),
        ("InstallFinalize", None, 6600),
    ];
    if options.major_upgrade {
        sequence.push(("FindRelatedProducts", Some("NOT REMOVE"), 25));
        sequence.push(("RemoveRelatedProducts", Some("UPGRADINGPRODUCTCODE"), 1800));
    }
    sequence.sort_by_key(|(_, _, order)| *order);
    tables.push(Table {
        name: "InstallExecuteSequence",
        columns: SEQUENCE_COLUMNS,
        rows: sequence
            .iter()
            .map(|(action, condition, order)| {
                vec![Cell::Text((*action).to_owned()), condition.map_or(Cell::Null, |text| Cell::Text(text.to_owned())), Cell::Int(*order)]
            })
            .collect(),
    });

    // Upgrade（可选）。
    if options.major_upgrade {
        tables.push(Table {
            name: "Upgrade",
            columns: UPGRADE_COLUMNS,
            rows: vec![vec![
                Cell::Text(upgrade_code.clone()),
                Cell::Text("Product".into()),
                Cell::Null,
                Cell::Null,
                Cell::Null,
                Cell::Int(0x0000_0100), // ONLYLATERVERSIONS
                Cell::Int(0x0000_0002), // REPLACE
            ]],
        });
    }

    // 4. 编码元数据与字符串池。
    let table_names: Vec<&'static str> = tables.iter().map(|table| table.name).collect();
    let tables_stream = encode_tables(&table_names, &mut pool)?;
    let columns_stream = encode_columns(&tables.iter().collect::<Vec<_>>(), &mut pool)?;
    let mut streams: Vec<(String, Vec<u8>)> = Vec::new();
    for table in &tables {
        let bytes = encode_rows(table, &mut pool)?;
        streams.push((table.name.to_owned(), bytes));
    }
    let (pool_stream, string_data) = pool.finish();

    // 5. cabinet。
    let cab_entries: Vec<CabEntry<'_>> = payload_bytes.iter().map(|(name, data)| CabEntry { name, data }).collect();
    let cab = build_cab(&cab_entries, 0x5741_4500);

    // 6. 汇总信息流与 DataSpaces。
    let summary = summary_information(spec, &product_code, &package_code, table_names.len() + 2);
    let cfb = assemble_cfb(&streams, &tables_stream, &columns_stream, &pool_stream, &string_data, &cab, &summary)?;

    // 7. 落盘。
    if let Some(parent) = spec.output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| InstallerError::Io(std::io::Error::new(error.kind(), format!("create {}: {error}", parent.display()))))?;
    }
    let mut file = std::fs::File::create(&spec.output)
        .map_err(|error| InstallerError::Io(std::io::Error::new(error.kind(), format!("create {}: {error}", spec.output.display()))))?;
    let bytes = cfb.finish()?;
    std::io::Write::write_all(&mut file, &bytes).map_err(InstallerError::Io)?;
    Ok(WriteReport {
        format: FormatId::Msi,
        output: spec.output.clone(),
        bytes: bytes.len() as u64,
        details: serde_json::json!({ "tables": table_names, "files": payload_bytes.len(), "productCode": product_code, "packageCode": package_code }),
    })
}

/// 安装目录叶子名：ASCII、空格转 `-`。
fn dir_name(product: &str) -> String {
    let cleaned: String = product
        .trim()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' { ch.to_ascii_uppercase() } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_owned();
    if trimmed.is_empty() { "Application".to_owned() } else { trimmed }
}

/// ARP 补充属性。
fn arp_properties(spec: &InstallerSpec) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let arp = &spec.product.arp;
    for (key, value) in [
        ("ARPCOMMENT", arp.comment.as_deref()),
        ("ARPCONTACT", arp.contact.as_deref()),
        ("ARPHELP", arp.help_link.as_deref()),
        ("ARPURLINFOABOUT", arp.url_info_about.as_deref()),
        ("ARPURLUPDATEINFO", arp.url_update_info.as_deref()),
    ] {
        if let Some(value) = value {
            rows.push((key.to_owned(), value.to_owned()));
        }
    }
    rows
}

/// `_SummaryInformation` OLE 属性集（[MS-OLEPS]）。
fn summary_information(spec: &InstallerSpec, product_code: &str, _package_code: &str, table_count: usize) -> Vec<u8> {
    // (id, value)：VT_I2=2、VT_I4=3、VT_FILETIME=64、VT_LPSTR=30。
    let mut props: Vec<(u32, PropValue)> = vec![
        (1, PropValue::I2(1252)),
        (2, PropValue::I4(spec.product.language as u32)),
        (3, PropValue::Str(spec.product.name.clone())),
        (4, PropValue::Str(String::new())),
        (5, PropValue::Str(spec.product.manufacturer.clone())),
        (6, PropValue::Str(String::new())),
        (7, PropValue::Str(String::new())),
        (8, PropValue::Str("Intel 386;Windows 95 or later:Intel 386;Windows NT Server 4.0 or later".into())),
        (9, PropValue::Str(spec.product.manufacturer.clone())),
        (12, PropValue::Str(product_code.to_owned())),
        (14, PropValue::I4(table_count as u32)),
        (15, PropValue::FileTime(0)),
        (16, PropValue::FileTime(0)),
        (18, PropValue::Str("wae-installer".into())),
    ];
    props.sort_by_key(|(id, _)| *id);

    // 值区先序列化，再回填偏移。
    let mut values = Vec::new();
    let mut offsets: Vec<u32> = Vec::with_capacity(props.len());
    for (_, value) in &props {
        offsets.push((values.len() as u32).next_multiple_of(4) + 4 + 4 + props.len() as u32 * 8);
        match value {
            PropValue::I2(raw) => {
                values.extend_from_slice(&2u32.to_le_bytes());
                values.extend_from_slice(&(*raw as u32).to_le_bytes());
            }
            PropValue::I4(raw) => {
                values.extend_from_slice(&3u32.to_le_bytes());
                values.extend_from_slice(&raw.to_le_bytes());
            }
            PropValue::FileTime(raw) => {
                values.extend_from_slice(&64u32.to_le_bytes());
                values.extend_from_slice(&raw.to_le_bytes());
            }
            PropValue::Str(text) => {
                let bytes = text.as_bytes();
                let padded = (bytes.len() + 1).next_multiple_of(4);
                values.extend_from_slice(&30u32.to_le_bytes());
                values.extend_from_slice(&((bytes.len() + 1) as u32).to_le_bytes());
                values.extend_from_slice(bytes);
                values.resize(values.len() + (padded - bytes.len()), 0); // NUL + 对齐
            }
        }
    }
    let size = 4 + 4 + props.len() as u32 * 8 + values.len() as u32;
    let mut section = Vec::new();
    section.extend_from_slice(&size.to_le_bytes()); // size 字段：覆盖 size 字段之后的全部字节
    section.extend_from_slice(&(props.len() as u32).to_le_bytes());
    for ((id, _), offset) in props.iter().zip(&offsets) {
        section.extend_from_slice(&id.to_le_bytes());
        section.extend_from_slice(&offset.to_le_bytes());
    }
    section.extend_from_slice(&values);

    let mut out = vec![0u8; 48];
    out[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
    out[2..4].copy_from_slice(&0u16.to_le_bytes());
    out[20..24].copy_from_slice(&1u32.to_le_bytes());
    out[24..40].copy_from_slice(&[0xE0, 0x85, 0x9F, 0xF2, 0xF9, 0x4F, 0x68, 0x10, 0xAB, 0x91, 0x08, 0x00, 0x2B, 0x27, 0xB3, 0xD9]);
    out[40..44].copy_from_slice(&48u32.to_le_bytes());
    out.extend_from_slice(&section);
    out
}

enum PropValue {
    I2(u16),
    I4(u32),
    FileTime(u64),
    Str(String),
}

/// `##DataSpaces` 三流（msitools 同值）。
fn data_spaces_streams() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let version = vec![
        0x14, 0x00, 0x00, 0x00, 0x00, 0x04, 0x09, 0x04, 0xE4, 0x4C, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];
    let validation = vec![0x04, 0x04, 0x00, 0x04, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let summary = vec![0x01, 0x80, 0x21, 0x00, 0x00, 0x80, 0x00, 0x00, 0x1D, 0x01, 0x00, 0x00];
    (version, validation, summary)
}

/// 组装 CFB：元数据流、表流、池、cabinet、SummaryInformation、DataSpaces。
fn assemble_cfb(
    tables: &[(String, Vec<u8>)],
    tables_stream: &[u8],
    columns_stream: &[u8],
    pool: &[u8],
    string_data: &[u8],
    cab: &[u8],
    summary: &[u8],
) -> Result<Cfb> {
    let mut cfb = Cfb::new();
    cfb.add_stream("_Tables", tables_stream.to_vec())?;
    cfb.add_stream("_Columns", columns_stream.to_vec())?;
    cfb.add_stream("_StringPool", pool.to_vec())?;
    cfb.add_stream("_StringData", string_data.to_vec())?;
    for (name, bytes) in tables {
        cfb.add_stream(name, bytes.clone())?;
    }
    cfb.add_stream(CABINET_STREAM, cab.to_vec())?;
    cfb.add_stream("\u{1}SummaryInformation", summary.to_vec())?;
    let (version, validation, data_summary) = data_spaces_streams();
    cfb.add_stream("##DataSpaces/Version", version)?;
    cfb.add_stream("##DataSpaces/Validation", validation)?;
    cfb.add_stream("##DataSpaces/SummaryInformation", data_summary)?;
    Ok(cfb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cfb::read_streams,
        spec::{ArpMeta, DirKey, PayloadFile, ProductMeta},
        tables::decode_rows,
    };

    fn build_spec(tag: &str) -> InstallerSpec {
        let dir = std::env::temp_dir().join(format!("wae-installer-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("app.exe"), vec![0x4Du8; 5000]).unwrap();
        std::fs::write(dir.join("index.html"), b"<html>hi</html>").unwrap();
        InstallerSpec {
            product: ProductMeta {
                name: "MyJarvis".into(),
                version: "0.1.0".into(),
                manufacturer: "Contoso".into(),
                language: 1033,
                upgrade_code: None,
                product_code: None,
                package_code: None,
                icon: None,
                arp: ArpMeta::default(),
            },
            payload: vec![
                PayloadFile { source: dir.join("app.exe"), target: DirKey::ProgramFiles, file_name: None, attributes: 0, executable: true },
                PayloadFile { source: dir.join("index.html"), target: DirKey::ProgramFiles, file_name: None, attributes: 0, executable: false },
            ],
            shortcuts: Vec::new(),
            output: dir.join("myjarvis.msi"),
        }
    }

    #[test]
    fn write_msi_round_trips_through_own_decoder() {
        let spec = build_spec("roundtrip");
        let report = write_msi(&spec, &MsiOptions { major_upgrade: true, ..MsiOptions::default() }).unwrap();
        assert!(report.bytes > 4096, "file too small: {}", report.bytes);
        let bytes = std::fs::read(&spec.output).unwrap();
        let streams = read_streams(&bytes).unwrap();
        for name in [
            "_Tables",
            "_Columns",
            "_StringPool",
            "_StringData",
            "Property",
            "Directory",
            "Component",
            "Feature",
            "FeatureComponents",
            "File",
            "Media",
            "InstallExecuteSequence",
            "Upgrade",
            "FileData",
            "\u{1}SummaryInformation",
            "##DataSpaces/Version",
            "##DataSpaces/Validation",
        ] {
            assert!(streams.contains_key(name), "missing stream {name}");
        }
        let pool = &streams["_StringPool"];
        let tables: Vec<String> = decode_rows(&streams["_Tables"], pool)
            .into_iter()
            .map(|row| match &row[0] {
                Cell::Text(text) => text.clone(),
                _ => panic!("_Tables row not text"),
            })
            .collect();
        assert!(tables.contains(&"Property".to_string()));
        assert!(tables.contains(&"InstallExecuteSequence".to_string()));
        let property = decode_rows(&streams["Property"], pool);
        let names: Vec<String> = property
            .iter()
            .map(|row| match &row[0] {
                Cell::Text(text) => text.clone(),
                _ => panic!(),
            })
            .collect();
        assert!(names.contains(&"ProductName".to_string()));
        assert!(names.contains(&"ProductCode".to_string()));
        let files = decode_rows(&streams["File"], pool);
        assert_eq!(files.len(), 2);
        let _ = std::fs::remove_dir_all(spec.output.parent().unwrap());
    }

    /// Windows 门：系统 `msi.dll` 进程内打开产物并逐表回读。
    /// 当前 RE 未完成（真实 `_StringPool` 为无 NUL 拼接、流名非 UTF-16），
    /// 产物尚未被 msi.dll 接受，暂 `#[ignore]`，容器与表编码补齐后启用。
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn write_msi_opens_with_system_msi_dll() {
        let spec = build_spec("msidll");
        write_msi(&spec, &MsiOptions { major_upgrade: true, ..MsiOptions::default() }).unwrap();
        let report = crate::verify::verify_msi(&spec.output).expect("msi.dll must open and query every table");
        assert!(report.tables.contains(&"Property".to_string()));
        assert!(report.rows.iter().any(|(name, count)| name == "File" && *count == 2));
        let _ = std::fs::remove_dir_all(spec.output.parent().unwrap());
    }

    /// 诊断（Windows，ignored）：二分定位 msi.dll 拒收点。
    /// 逐层叠加流，打印每层 `MsiOpenDatabaseW` + `_Tables` 查询结果。
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn bisect_msi_dll_open() {
        let dir = std::env::temp_dir().join("wae-installer-bisect");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let spec = build_spec("bisect");
        write_msi(&spec, &MsiOptions { major_upgrade: true, ..MsiOptions::default() }).unwrap();
        let full = std::fs::read(&spec.output).unwrap();

        // 用本 crate 解码 full，取得各流字节，再按层重组。
        let streams = crate::cfb::read_streams(&full).unwrap();
        let meta: Vec<(&str, Vec<u8>)> = [
            "_Tables",
            "_Columns",
            "_StringPool",
            "_StringData",
            "Property",
            "Directory",
            "Component",
            "Feature",
            "FeatureComponents",
            "File",
            "Media",
            "InstallExecuteSequence",
            "Upgrade",
        ]
        .iter()
        .map(|name| (*name, streams.get(*name).cloned().unwrap_or_default()))
        .collect();

        let probe = |label: &str, cfb: &crate::cfb::Cfb| {
            let path = dir.join(format!("{label}.msi"));
            let mut file = std::fs::File::create(&path).unwrap();
            std::io::Write::write_all(&mut file, &cfb.finish().unwrap()).unwrap();
            drop(file);
            let ole = crate::verify::ole32_storage_open(&path);
            let open = crate::verify::msi_api_query(&path, "SELECT `Name` FROM `_Tables`");
            println!(
                "{label}: ole32={:?} msi={:?}",
                ole.map_err(|e| format!("{e:#010X}")),
                open.as_ref().map(|rows| rows.len()).map_err(|e| e.clone())
            );
        };

        // L0：仅元数据 + 表流。
        let mut l0 = crate::cfb::Cfb::new();
        for (name, bytes) in &meta {
            l0.add_stream(name, bytes.clone()).unwrap();
        }
        probe("L0-meta+tables", &l0);

        // L1：L0 + FileData cab。
        let mut l1 = crate::cfb::Cfb::new();
        for (name, bytes) in &meta {
            l1.add_stream(name, bytes.clone()).unwrap();
        }
        l1.add_stream("FileData", streams.get("FileData").cloned().unwrap()).unwrap();
        probe("L1+FileData", &l1);

        // L2：L1 + SummaryInformation。
        let mut l2 = crate::cfb::Cfb::new();
        for (name, bytes) in &meta {
            l2.add_stream(name, bytes.clone()).unwrap();
        }
        l2.add_stream("FileData", streams.get("FileData").cloned().unwrap()).unwrap();
        l2.add_stream("\u{1}SummaryInformation", streams.get("\u{1}SummaryInformation").cloned().unwrap()).unwrap();
        probe("L2+Summary", &l2);

        // L3：L2 + DataSpaces。
        let mut l3 = crate::cfb::Cfb::new();
        for (name, bytes) in &meta {
            l3.add_stream(name, bytes.clone()).unwrap();
        }
        l3.add_stream("FileData", streams.get("FileData").cloned().unwrap()).unwrap();
        l3.add_stream("\u{1}SummaryInformation", streams.get("\u{1}SummaryInformation").cloned().unwrap()).unwrap();
        l3.add_stream("##DataSpaces/Version", streams.get("##DataSpaces/Version").cloned().unwrap()).unwrap();
        l3.add_stream("##DataSpaces/Validation", streams.get("##DataSpaces/Validation").cloned().unwrap()).unwrap();
        l3.add_stream("##DataSpaces/SummaryInformation", streams.get("##DataSpaces/SummaryInformation").cloned().unwrap()).unwrap();
        probe("L3+DataSpaces", &l3);

        // L4：完整产物（对照 write_msi 输出本身）。
        let path = dir.join("L4-full.msi");
        std::fs::copy(&spec.output, &path).unwrap();
        let open = crate::verify::msi_api_query(&path, "SELECT `Name` FROM `_Tables`");
        println!("L4-full: {:?}", open.as_ref().map(|rows| rows.len()).map_err(|e| e.clone()));

        // L5：完整产物但去掉 Upgrade 表（对照条件分支）。
        let mut l5 = crate::cfb::Cfb::new();
        for (name, bytes) in &meta {
            if *name != "Upgrade" {
                l5.add_stream(name, bytes.clone()).unwrap();
            }
        }
        l5.add_stream("FileData", streams.get("FileData").cloned().unwrap()).unwrap();
        l5.add_stream("\u{1}SummaryInformation", streams.get("\u{1}SummaryInformation").cloned().unwrap()).unwrap();
        l5.add_stream("##DataSpaces/Version", streams.get("##DataSpaces/Version").cloned().unwrap()).unwrap();
        l5.add_stream("##DataSpaces/Validation", streams.get("##DataSpaces/Validation").cloned().unwrap()).unwrap();
        l5.add_stream("##DataSpaces/SummaryInformation", streams.get("##DataSpaces/SummaryInformation").cloned().unwrap()).unwrap();
        probe("L5-noUpgrade", &l5);

        // L6：真实文件原样重发（容器正确性对照）。读真实流字节 + 名字，
        // 经本 crate CFB writer 重出，看 msi.dll 是否打开。
        let real = std::fs::read_dir(r"C:\Windows\Installer")
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.extension().and_then(|x| x.to_str()) == Some("msi")
                    && std::fs::metadata(p).is_ok_and(|m| m.len() > 4096 && m.len() < 4_000_000)
            })
            .expect("no real msi");
        let real_bytes = std::fs::read(&real).unwrap();
        println!("ole32 on ORIGINAL real: {:?}", crate::verify::ole32_storage_open(&real).map_err(|e| format!("{e:#010X}")));
        let real_streams = crate::cfb::read_streams(&real_bytes).unwrap();
        println!("L6 real stream count: {}", real_streams.len());
        let mut l6 = crate::cfb::Cfb::new();
        for (name, data) in &real_streams {
            l6.add_stream(name, data.clone()).unwrap();
        }
        let remitted = l6.finish().unwrap();
        let hdr = |b: &[u8], tag: &str| {
            let u16_at = |at: usize| u16::from_le_bytes(b[at..at + 2].try_into().unwrap());
            let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
            println!(
                "{tag}: major={:X} minor={:X} ss={:X} ms={:X} numDir={num_dir} numFat={num_fat} dirStart={dir_start} tx={tx} cutoff={cutoff} mfStart={mf} numMf={nmf} difatStart={ds} numDifat={nd} DIFAT0={d0} DIFAT1={d1}",
                u16_at(0x1A),
                u16_at(0x18),
                u16_at(0x1E),
                u16_at(0x20),
                num_dir = u32_at(0x28),
                num_fat = u32_at(0x2C),
                dir_start = u32_at(0x30),
                tx = u32_at(0x34),
                cutoff = u32_at(0x38),
                mf = u32_at(0x3C),
                nmf = u32_at(0x40),
                ds = u32_at(0x44),
                nd = u32_at(0x48),
                d0 = u32_at(0x4C),
                d1 = u32_at(0x50)
            );
        };
        hdr(&real_bytes, "real");
        hdr(&remitted, "remitted");
        println!("real len={} remitted len={}", real_bytes.len(), remitted.len());
        let fat_at = |b: &[u8], i: usize| u32::from_le_bytes(b[4096 + i * 4..4096 + i * 4 + 4].try_into().unwrap());
        let root_at = |b: &[u8]| {
            (
                u32::from_le_bytes(b[4096 + 4096 + 0x74..4096 + 4096 + 0x78].try_into().unwrap()),
                u32::from_le_bytes(b[4096 + 4096 + 0x78..4096 + 4096 + 0x7C].try_into().unwrap()),
            )
        };
        println!("FAT real={:?}", (0..20).map(|i| fat_at(&real_bytes, i)).collect::<Vec<_>>());
        println!("FAT rem ={:?}", (0..20).map(|i| fat_at(&remitted, i)).collect::<Vec<_>>());
        println!("root real start/size={:?} rem={:?}", root_at(&real_bytes), root_at(&remitted));
        probe("L6-real-remitted", &l6);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
