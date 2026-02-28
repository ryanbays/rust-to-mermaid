use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use tree_sitter::{Node, Parser};
use walkdir::WalkDir;

#[derive(Default)]
pub struct Class {
    pub name: String,
    pub file: String,
    pub stereotype: Option<String>,
    /// First doc-comment line for this type, if present.
    pub doc: Option<String>,
    pub fields: Vec<String>,
    pub methods: Vec<String>,
    pub relationships: BTreeSet<Relationship>,
    pub trait_impls: BTreeSet<String>,
}

pub struct FunctionInfo {
    pub name: String,
    /// First doc-comment line for this function, if present.
    pub doc: Option<String>,
    /// Parameter list as rendered strings ("x: i32", "y: String", ...)
    pub params: Vec<String>,
    /// Return type as rendered string ("usize", "Result<T>", ...)
    pub ret: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct Relationship {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub label: Option<String>,
}

pub struct ParsedProject {
    pub classes: HashMap<String, Class>,
    pub file_functions_main: HashMap<String, Vec<FunctionInfo>>,
    pub file_functions_tests: HashMap<String, Vec<FunctionInfo>>,
}

/// Public API: parse all Rust files under `src_dir` and build our model.
pub fn parse_project(src_dir: &Path) -> anyhow::Result<ParsedProject> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;

    let mut classes: HashMap<String, Class> = HashMap::new();
    let mut file_functions_main: HashMap<String, Vec<FunctionInfo>> = HashMap::new();
    let mut file_functions_tests: HashMap<String, Vec<FunctionInfo>> = HashMap::new();
    let mut local_types = HashSet::new();

    // FIRST PASS: collect all local types
    for file in rust_files(src_dir) {
        let content = fs::read_to_string(&file)?;
        let tree = parser.parse(&content, None).unwrap();
        collect_type_names(tree.root_node(), &content, &mut local_types);
    }

    // SECOND PASS: extract items
    for file in rust_files(src_dir) {
        let content = fs::read_to_string(&file)?;
        let tree = parser.parse(&content, None).unwrap();

        let rel_path = file
            .strip_prefix(src_dir)
            .unwrap_or(&file)
            .to_string_lossy()
            .to_string();

        let file_module = Path::new(&rel_path)
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

    Ok(ParsedProject {
        classes,
        file_functions_main,
        file_functions_tests,
    })
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
                    let params = extract_function_params(child, src);
                    let ret = extract_function_return_type(child, src);

                    let info = FunctionInfo {
                        name: func_name,
                        doc,
                        params,
                        ret,
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

/// Extract a function's parameters as "name: Type" or "Type" strings.
fn extract_function_params(func: Node, src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let mut params = Vec::new();

    if let Some(param_list) = func.child_by_field_name("parameters") {
        let mut cursor = param_list.walk();
        for child in param_list.children(&mut cursor) {
            match child.kind() {
                // regular param: e.g. `x: i32`
                "parameter" => {
                    let name = child
                        .child_by_field_name("pattern")
                        .and_then(|n| n.utf8_text(bytes).ok())
                        .map(|s| s.trim().to_string());
                    let ty = child
                        .child_by_field_name("type")
                        .and_then(|n| n.utf8_text(bytes).ok())
                        .map(|s| s.trim().to_string());

                    let rendered = match (name, ty) {
                        (Some(n), Some(t)) => format!("{n}: {t}"),
                        (Some(n), None) => n,
                        (None, Some(t)) => t,
                        (None, None) => continue,
                    };
                    params.push(rendered);
                }
                // `self`, `&self`, `&mut self`
                "self_parameter" => {
                    if let Ok(text) = child.utf8_text(bytes) {
                        params.push(text.trim().to_string());
                    }
                }
                // ignore commas etc.
                _ => {}
            }
        }
    }

    params
}

/// Extract a function's return type as a string, if present.
fn extract_function_return_type(func: Node, src: &str) -> Option<String> {
    let bytes = src.as_bytes();
    let ret_node = func.child_by_field_name("return_type")?;
    // In tree-sitter-rust, `return_type` usually looks like `-> Type`
    // So strip the leading `->` if present.
    let text = ret_node.utf8_text(bytes).ok()?.trim().to_string();
    let stripped = text
        .strip_prefix("->")
        .map(|s| s.trim().to_string())
        .unwrap_or(text);
    if stripped.is_empty() {
        None
    } else {
        Some(stripped)
    }
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
