use anyhow::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use crate::parse::{Class, FunctionInfo, ParsedProject};

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

/// Public entry point: generate both main and test diagrams into the crate root.
///
/// `config` controls the title / layout / theme of the generated Mermaid files.
pub fn generate_diagrams_with_config(config: &DiagramConfig<'_>) -> Result<()> {
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

    // Use parser module
    let ParsedProject {
        classes,
        file_functions_main,
        file_functions_tests,
    } = crate::parser::parse_project(src_path)?;

    // Group classes by main/test file
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

    render_functions_namespaces(&mut mermaid_main, &file_functions_main);

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

    render_functions_namespaces(&mut mermaid_tests, &file_functions_tests);

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

fn render_functions_namespaces(out: &mut String, files: &BTreeMap<String, Vec<FunctionInfo>>) {
    for (file_module, funcs) in files {
        let ns_title = format!("{}.rs", file_module);
        out.push_str(&format!("    namespace `{}` {{\n", ns_title));
        out.push_str(&format!("        class `{}_functions` {{\n", file_module));
        for f in funcs {
            let param_list = if f.params.is_empty() {
                "".to_string()
            } else {
                f.params.join(", ")
            };
            let sig = if let Some(ret) = &f.ret {
                format!("{} {}({})", ret, f.name, param_list)
            } else {
                format!("{}({})", f.name, param_list)
            };
            if let Some(doc) = &f.doc {
                out.push_str(&format!("            {} {}\n", sig, doc));
            } else {
                out.push_str(&format!("            {}\n", sig));
            }
        }
        out.push_str("        }\n");
        out.push_str("    }\n");
    }
}

fn is_test_file(file_module: &str) -> bool {
    file_module.starts_with("tests")
        || file_module.contains("/tests/")
        || file_module.ends_with("_test")
        || file_module.ends_with("_tests")
}
