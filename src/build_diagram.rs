use anyhow::Result;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
};
use tree_sitter::{Node, Parser};
use walkdir::WalkDir;

#[derive(Default)]
struct Class {
    name: String,
    file: String,
    stereotype: Option<String>,
    /// First doc-comment line for this type, if present.
    doc: Option<String>,
    fields: Vec<String>,
    methods: Vec<String>,
    relationships: BTreeSet<Relationship>,
    trait_impls: BTreeSet<String>,
}

struct FunctionInfo {
    name: String,
    /// First doc-comment line for this function, if present.
    doc: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct Relationship {
    source: String,
    target: String,
    edge_type: String,
    label: Option<String>,
}

impl Class {
    fn render(&self) -> (String, Option<String>) {
        let mut s = String::new();

        s.push_str(&format!("        class `{}` {{\n", self.name));

        if let Some(st) = &self.stereotype {
            s.push_str(&format!("            <<{}>>\n", st));
        }

        s.push_str(&format!("            <<{}>>\n", self.file));

        for f in &self.fields {
            s.push_str(&format!("            {}\n", f));
        }

        for m in &self.methods {
            s.push_str(&format!("            {}()\n", m));
        }

        s.push_str("        }\n");

        let note = self
            .doc
            .as_ref()
            .map(|doc| format!("note for `{}` \"{}\"\n", self.name, doc));

        (s, note)
    }
}

/// Options controlling how the Mermaid diagrams are rendered.
pub struct DiagramConfig<'a> {
    pub main_title: &'a str,
    pub tests_title: &'a str,
    pub layout: &'a str,
    pub theme: &'a str,
    pub elk_node_placement: &'a str,
    /// Path to the Rust source directory to scan.
    pub src_dir: &'a Path,
    /// Directory where the generated Mermaid files will be written.
    pub out_dir: &'a Path,
}

fn default_manifest_dir() -> PathBuf {
    env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

impl<'a> Default for DiagramConfig<'a> {
    fn default() -> Self {
        Self {
            main_title: "Project",
            tests_title: "Project Tests",
            layout: "elk",
            theme: "dark",
            elk_node_placement: "BRANDES_KOEPF",
            src_dir: Path::new("src"),
            out_dir: Path::new("diagrams"),
        }
    }
}

/// Public entry point: generate both main and test diagrams into the crate root.
///
/// `config` controls the title / layout / theme of the generated Mermaid files.
pub fn generate_diagrams_with_config(config: &DiagramConfig<'_>) -> Result<()> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;

    let manifest_dir = default_manifest_dir();

    // If src_dir/out_dir in config are empty (from Default), fall back to
    // manifest-based paths.
    let src_path_buf;
    let out_path_buf;

    let src_path: &Path = if config.src_dir.as_os_str().is_empty() {
        src_path_buf = manifest_dir.join("src");
        &src_path_buf
    } else {
        config.src_dir
    };

    let out_path: &Path = if config.out_dir.as_os_str().is_empty() {
        out_path_buf = manifest_dir.clone();
        &out_path_buf
    } else {
        config.out_dir
    };

    let mut classes: HashMap<String, Class> = HashMap::new();
    let mut file_functions_main: HashMap<String, Vec<FunctionInfo>> = HashMap::new();
    let mut file_functions_tests: HashMap<String, Vec<FunctionInfo>> = HashMap::new();
    let mut local_types = HashSet::new();

    // FIRST PASS: collect all local types
    for file in rust_files(src_path) {
        let content = fs::read_to_string(&file)?;
        let tree = parser.parse(&content, None).unwrap();
        collect_type_names(tree.root_node(), &content, &mut local_types);
    }

    // SECOND PASS: extract
    for file in rust_files(src_path) {
        let content = fs::read_to_string(&file)?;
        let tree = parser.parse(&content, None).unwrap();

        let rel_path = file
            .strip_prefix(src_path)
            .unwrap_or(&file)
            .to_string_lossy()
            .to_string();

        let file_module = std::path::Path::new(&rel_path)
            .with_extension("")
            .to_string_lossy()
            .to_string();

        extract_items(
            tree.root_node(),
            &content,
            &rel_path,
            &file_module,
            &local_types,
            &mut classes,
            &mut file_functions_main,
            &mut file_functions_tests,
        );
    }

    let mut files_main: BTreeMap<String, Vec<&Class>> = BTreeMap::new();
    let mut files_test: BTreeMap<String, Vec<&Class>> = BTreeMap::new();

    for class in classes.values() {
        if is_test_file(&class.file) {
            files_test
                .entry(class.file.clone())
                .or_default()
                .push(class);
        } else {
            files_main
                .entry(class.file.clone())
                .or_default()
                .push(class);
        }
    }

    let mut notes_main: Vec<String> = Vec::new();
    let mut notes_test: Vec<String> = Vec::new();

    let mut rendered_edges_main = BTreeSet::new();
    let mut rendered_edges_test = BTreeSet::new();

    for class in classes.values() {
        let is_test = is_test_file(&class.file);
        let edges = if is_test {
            &mut rendered_edges_test
        } else {
            &mut rendered_edges_main
        };

        for rel in &class.relationships {
            if let Some(label) = &rel.label {
                edges.insert(format!(
                    "    {src} {edge} {tgt} : {label}\n",
                    src = rel.source,
                    edge = rel.edge_type,
                    tgt = rel.target,
                    label = label
                ));
            } else {
                edges.insert(format!(
                    "    {src} {edge} {tgt}\n",
                    src = rel.source,
                    edge = rel.edge_type,
                    tgt = rel.target
                ));
            }
        }

        for trait_impl in &class.trait_impls {
            edges.insert(format!("    {} <|.. {}\n", trait_impl, class.name));
        }
    }

    let mut mermaid_main = String::new();
    let mut mermaid_tests = String::new();

    mermaid_main.push_str(&format!(
        "---\n\
config:\n  title: {title}\n  layout: {layout}\n  theme: {theme}\n  elk:\n    mergeEdges: true\n    nodePlacementStrategy: {elk_node_placement}\n---\n",
        title = config.main_title,
        layout = config.layout,
        theme = config.theme,
        elk_node_placement = config.elk_node_placement,
    ));
    mermaid_main.push_str("classDiagram\n    direction TB\n");

    for (file_module, class_list) in &files_main {
        let ns_title = format!("{}.rs", file_module);
        mermaid_main.push_str(&format!("    namespace `{}` {{\n", ns_title));
        for class in class_list {
            let (class_str, note_opt) = class.render();
            mermaid_main.push_str(&class_str);
            if let Some(note) = note_opt {
                notes_main.push(note);
            }
        }
        mermaid_main.push_str("    }\n");
    }

    for (file_module, funcs) in &file_functions_main {
        let ns_title = format!("{}.rs", file_module);
        mermaid_main.push_str(&format!("    namespace `{}` {{\n", ns_title));
        mermaid_main.push_str(&format!("        class `{}_functions` {{\n", file_module));
        for f in funcs {
            if let Some(doc) = &f.doc {
                mermaid_main.push_str(&format!("            {}() {}\n", f.name, doc));
            } else {
                mermaid_main.push_str(&format!("            {}()\n", f.name));
            }
        }
        mermaid_main.push_str("        }\n");
        mermaid_main.push_str("    }\n");
    }

    for edge in rendered_edges_main {
        mermaid_main.push_str(&edge);
    }

    for note in notes_main {
        mermaid_main.push_str(&note);
    }

    mermaid_tests.push_str(&format!(
        "---\n\
config:\n  title: {title}\n  layout: {layout}\n  theme: {theme}\n  elk:\n    mergeEdges: true\n    nodePlacementStrategy: {elk_node_placement}\n---\n",
        title = config.tests_title,
        layout = config.layout,
        theme = config.theme,
        elk_node_placement = config.elk_node_placement,
    ));
    mermaid_tests.push_str("classDiagram\n    direction TB\n");

    for (file_module, class_list) in &files_test {
        let ns_title = format!("{}.rs", file_module);
        mermaid_tests.push_str(&format!("    namespace `{}` {{\n", ns_title));
        for class in class_list {
            let (class_str, note_opt) = class.render();
            mermaid_tests.push_str(&class_str);
            if let Some(note) = note_opt {
                notes_test.push(note);
            }
        }
        mermaid_tests.push_str("    }\n");
    }

    for (file_module, funcs) in &file_functions_tests {
        let ns_title = format!("{}.rs", file_module);
        mermaid_tests.push_str(&format!("    namespace `{}` {{\n", ns_title));
        mermaid_tests.push_str(&format!("        class `{}_functions` {{\n", file_module));
        for f in funcs {
            if let Some(doc) = &f.doc {
                mermaid_tests.push_str(&format!("            {}() {}\n", f.name, doc));
            } else {
                mermaid_tests.push_str(&format!("            {}()\n", f.name));
            }
        }
        mermaid_tests.push_str("        }\n");
        mermaid_tests.push_str("    }\n");
    }

    for edge in rendered_edges_test {
        mermaid_tests.push_str(&edge);
    }

    for note in notes_test {
        mermaid_tests.push_str(&note);
    }

    fs::create_dir_all(out_path)?;

    fs::write(out_path.join("diagram.mmd"), mermaid_main)?;
    fs::write(out_path.join("diagram_tests.mmd"), mermaid_tests)?;

    Ok(())
}

/// Backwards-compatible helper using default config.
pub fn generate_diagrams() -> Result<()> {
    // Build a real config with owned paths, then pass references.
    let manifest_dir = default_manifest_dir();
    let src_dir_buf = manifest_dir.join("src");
    let out_dir_buf = manifest_dir;

    let cfg = DiagramConfig {
        main_title: "Project",
        tests_title: "Project Tests",
        layout: "elk",
        theme: "dark",
        elk_node_placement: "BRANDES_KOEPF",
        src_dir: &src_dir_buf,
        out_dir: &out_dir_buf,
    };

    generate_diagrams_with_config(&cfg)
}

fn rust_files(path: &Path) -> Vec<PathBuf> {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|e| e == "rs").unwrap_or(false))
        .map(|e| e.path().to_path_buf())
        .collect()
}

fn collect_type_names(node: Node, src: &str, out: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct_item" | "enum_item" | "trait_item" => {
                if let Some(name) = child.child_by_field_name("name") {
                    out.insert(name.utf8_text(src.as_bytes()).unwrap().to_string());
                }
            }
            _ => {}
        }
        collect_type_names(child, src, out);
    }
}

fn has_test_attribute(node: Node, src: &str) -> bool {
    let bytes = src.as_bytes();

    let mut cur = match node.prev_sibling() {
        Some(n) => n,
        None => return false,
    };

    loop {
        match cur.kind() {
            "attribute_item" | "attribute" => {
                if let Ok(text) = cur.utf8_text(bytes) {
                    if text.contains("#[test]") || text.contains("test]") {
                        return true;
                    }
                }
            }
            "line_comment" | "block_comment" => {}
            _ => {
                let text = cur.utf8_text(bytes).unwrap_or("");
                if !text.trim().is_empty() {
                    break;
                }
            }
        }

        match cur.prev_sibling() {
            Some(prev) => cur = prev,
            None => break,
        }
    }

    false
}

fn extract_items(
    node: Node,
    src: &str,
    file_name: &str,
    file_module: &str,
    local_types: &HashSet<String>,
    classes: &mut HashMap<String, Class>,
    file_functions_main: &mut HashMap<String, Vec<FunctionInfo>>,
    file_functions_tests: &mut HashMap<String, Vec<FunctionInfo>>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct_item" => {
                let name = child
                    .child_by_field_name("name")
                    .unwrap()
                    .utf8_text(src.as_bytes())
                    .unwrap()
                    .to_string();

                let doc = leading_doc_comment(child, src);

                let class = classes.entry(name.clone()).or_insert_with(|| Class {
                    name: name.clone(),
                    file: file_module.into(),
                    stereotype: Some("struct".into()),
                    doc,
                    ..Default::default()
                });

                if let Some(body) = child.child_by_field_name("body") {
                    let mut c = body.walk();
                    for field in body.children(&mut c) {
                        if field.kind() == "field_declaration" {
                            let field_name = field
                                .child_by_field_name("name")
                                .and_then(|n| n.utf8_text(src.as_bytes()).ok())
                                .map(|s| s.to_string());

                            if let Some(ref fname) = field_name {
                                class.fields.push(fname.clone());
                            }

                            if let Some(ftype) = field.child_by_field_name("type") {
                                for ty in extract_type_identifiers(ftype, src) {
                                    if local_types.contains(&ty) {
                                        let edge = if ftype.kind() == "reference_type" {
                                            "o--"
                                        } else {
                                            "*--"
                                        };

                                        let label = field_name
                                            .as_ref()
                                            .map(|fname| format!("{} {}", ty, fname));

                                        class.relationships.insert(Relationship {
                                            source: class.name.clone(),
                                            target: ty,
                                            edge_type: edge.into(),
                                            label,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            "enum_item" => {
                let name = child
                    .child_by_field_name("name")
                    .unwrap()
                    .utf8_text(src.as_bytes())
                    .unwrap()
                    .to_string();

                let doc = leading_doc_comment(child, src);

                let class = classes.entry(name.clone()).or_insert_with(|| Class {
                    name: name.clone(),
                    file: file_module.into(),
                    stereotype: Some("enum".into()),
                    doc,
                    ..Default::default()
                });

                if let Some(body) = child.child_by_field_name("body") {
                    let mut c = body.walk();
                    for variant in body.children(&mut c) {
                        if variant.kind() == "enum_variant" {
                            let variant_name = variant
                                .child_by_field_name("name")
                                .and_then(|n| n.utf8_text(src.as_bytes()).ok())
                                .map(|s| s.to_string());

                            if let Some(ref vname) = variant_name {
                                class.fields.push(vname.clone());
                            }

                            if let Some(vbody) = variant.child_by_field_name("body") {
                                let mut vc = vbody.walk();
                                for field in vbody.children(&mut vc) {
                                    if field.kind() == "field_declaration" {
                                        let field_name = field
                                            .child_by_field_name("name")
                                            .and_then(|n| n.utf8_text(src.as_bytes()).ok())
                                            .map(|s| s.to_string());

                                        if let Some(ftype) = field.child_by_field_name("type") {
                                            for ty in extract_type_identifiers(ftype, src) {
                                                if local_types.contains(&ty) {
                                                    let edge = if ftype.kind() == "reference_type" {
                                                        "o--"
                                                    } else {
                                                        "*--"
                                                    };

                                                    let label = match (&variant_name, &field_name) {
                                                        (Some(vn), Some(fn_)) => {
                                                            Some(format!("{} {}::{}", ty, vn, fn_))
                                                        }
                                                        (Some(vn), None) => {
                                                            Some(format!("{} {}", ty, vn))
                                                        }
                                                        _ => None,
                                                    };

                                                    class.relationships.insert(Relationship {
                                                        source: class.name.clone(),
                                                        target: ty,
                                                        edge_type: edge.into(),
                                                        label,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            "function_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let func_name = name_node.utf8_text(src.as_bytes()).unwrap().to_string();
                    let doc = leading_doc_comment(child, src);

                    let info = FunctionInfo {
                        name: func_name,
                        doc,
                    };

                    if has_test_attribute(child, src) {
                        file_functions_tests
                            .entry(file_module.into())
                            .or_default()
                            .push(info);
                    } else {
                        file_functions_main
                            .entry(file_module.into())
                            .or_default()
                            .push(info);
                    }
                }
            }

            _ => {}
        }

        extract_items(
            child,
            src,
            file_name,
            file_module,
            local_types,
            classes,
            file_functions_main,
            file_functions_tests,
        );
    }
}

fn extract_type_identifiers(node: Node, src: &str) -> Vec<String> {
    let mut result = vec![];
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" {
            result.push(child.utf8_text(src.as_bytes()).unwrap().to_string());
        }
        result.extend(extract_type_identifiers(child, src));
    }
    result
}

/// Extract the first-line doc comment (`/// ...`) preceding `item`, if any.
fn leading_doc_comment(item: Node, src: &str) -> Option<String> {
    let bytes = src.as_bytes();
    let mut cursor = item.prev_sibling()?;

    let mut last_doc_line: Option<String> = None;

    loop {
        match cursor.kind() {
            "line_comment" => {
                let text = cursor.utf8_text(bytes).ok()?;
                if let Some(stripped) = text.strip_prefix("///") {
                    last_doc_line = Some(stripped.trim().to_string());
                } else {
                    break;
                }
            }
            _ => {
                let text = cursor.utf8_text(bytes).ok()?;
                if !text.trim().is_empty() {
                    break;
                }
            }
        }

        match cursor.prev_sibling() {
            Some(prev) => cursor = prev,
            None => break,
        }
    }

    last_doc_line
}

fn is_test_file(file_module: &str) -> bool {
    file_module.starts_with("tests")
        || file_module.contains("/tests/")
        || file_module.ends_with("_test")
        || file_module.ends_with("_tests")
}
