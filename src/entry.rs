use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use ratatui::style::Color;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_executable: bool,
}

// All icons use explicit \u{XXXX} Nerd Font (v2) codepoints
const FOLDER:    &str = "\u{F07B}";
const FILE:      &str = "\u{F15B}";
const RUST:      &str = "\u{E7A8}";
const JS:        &str = "\u{E74E}";
const TS:        &str = "\u{E628}";
const JSON:      &str = "\u{E60B}";
const HTML:      &str = "\u{E736}";
const CSS:       &str = "\u{E749}";
const SCSS:      &str = "\u{E603}";
const PYTHON:    &str = "\u{E606}";
const GO:        &str = "\u{E627}";
const C:         &str = "\u{E61E}";
const CPP:       &str = "\u{E61D}";
const JAVA:      &str = "\u{E738}";
const RUBY:      &str = "\u{E21E}";
const SHELL:     &str = "\u{F489}";
const MARKDOWN:  &str = "\u{E609}";
const TOML:      &str = "\u{E6B2}";
const YAML:      &str = "\u{E60A}";
const SQL:       &str = "\u{F1C0}";
const IMAGE:     &str = "\u{F1C5}";
const VIDEO:     &str = "\u{F03D}";
const AUDIO:     &str = "\u{F001}";
const PDF:       &str = "\u{F1C1}";
const ARCHIVE:   &str = "\u{F1C6}";
const LOCK:      &str = "\u{F023}";
const COG:       &str = "\u{F013}";
const GIT:       &str = "\u{E702}";
const DOCKER:    &str = "\u{E7B0}";
const NODE:      &str = "\u{E718}";
const TEXT:      &str = "\u{F0F6}";
const LEGAL:     &str = "\u{F0E3}";
const BINARY:    &str = "\u{F471}";
const LIB:       &str = "\u{F1B2}";
const RUN:       &str = "\u{F0E7}";
const SWIFT:     &str = "\u{E755}";
const KOTLIN:    &str = "\u{E634}";
const LUA:       &str = "\u{E620}";
const VIM:       &str = "\u{E62B}";
const NIX:       &str = "\u{F313}";
const TERRAFORM: &str = "\u{E69A}";
const FONT:      &str = "\u{F031}";
const KEY:       &str = "\u{F805}";
const CSV:       &str = "\u{F1C3}";
const NETWORK:   &str = "\u{F0AC}";

pub fn icon_for_name(name: &str) -> (&'static str, Color) {
    match name.to_lowercase().as_str() {
        "cargo.toml"                                       => return (RUST,   Color::Rgb(222, 165, 132)),
        "cargo.lock"                                       => return (LOCK,   Color::Rgb(183, 183, 183)),
        "package.json" | "package-lock.json"               => return (NODE,   Color::Rgb(203, 120,  50)),
        ".gitignore" | ".gitmodules" | ".gitattributes"    => return (GIT,    Color::Rgb(241,  80,  47)),
        "dockerfile" | "docker-compose.yml"
        | "docker-compose.yaml"                            => return (DOCKER, Color::Rgb(  1, 135, 201)),
        "makefile" | "gnumakefile"                         => return (COG,    Color::Rgb(111, 193,  44)),
        "license" | "licence"                              => return (LEGAL,  Color::Rgb(240, 214,  83)),
        _ => {}
    }

    let ext = std::path::Path::new(name).extension().and_then(|s| s.to_str()).unwrap_or("");
    match ext {
        "rs"                              => (RUST,      Color::Rgb(222, 165, 132)),
        "js" | "mjs" | "cjs"             => (JS,        Color::Rgb(240, 214,  83)),
        "ts" | "mts" | "cts"             => (TS,        Color::Rgb( 49, 120, 198)),
        "jsx"                             => (JS,        Color::Rgb( 97, 218, 251)),
        "tsx"                             => (TS,        Color::Rgb( 49, 120, 198)),
        "json"                            => (JSON,      Color::Rgb(240, 214,  83)),
        "html" | "htm"                    => (HTML,      Color::Rgb(228,  79,  38)),
        "css"                             => (CSS,       Color::Rgb( 38, 143, 222)),
        "scss" | "sass"                   => (SCSS,      Color::Rgb(204, 102, 153)),
        "py" | "pyi"                      => (PYTHON,    Color::Rgb( 55, 118, 171)),
        "go"                              => (GO,        Color::Rgb(  1, 173, 216)),
        "c" | "h"                         => (C,         Color::Rgb( 85, 170, 255)),
        "cpp" | "cc" | "cxx" | "hpp"     => (CPP,       Color::Rgb(243,  75, 125)),
        "java"                            => (JAVA,      Color::Rgb(176, 114,  25)),
        "rb"                              => (RUBY,      Color::Rgb(204,  52,  45)),
        "swift"                           => (SWIFT,     Color::Rgb(240,  88,  35)),
        "kt" | "kts"                      => (KOTLIN,    Color::Rgb(127,  82, 255)),
        "lua"                             => (LUA,       Color::Rgb( 66, 132, 204)),
        "vim" | "vimrc" | "nvim"          => (VIM,       Color::Rgb( 41, 154,  77)),
        "nix"                             => (NIX,       Color::Rgb( 80, 120, 200)),
        "tf" | "tfvars" | "tfstate"       => (TERRAFORM, Color::Rgb( 95,  64, 191)),
        "sh" | "bash" | "zsh" | "fish"   => (SHELL,     Color::Rgb(121, 182, 122)),
        "md" | "mdx"                      => (MARKDOWN,  Color::Rgb( 66, 165, 245)),
        "toml"                            => (TOML,      Color::Rgb(156, 175, 183)),
        "yaml" | "yml"                    => (YAML,      Color::Rgb(204, 204, 204)),
        "ini" | "cfg" | "conf"            => (COG,       Color::Rgb(180, 180, 120)),
        "env"                             => (KEY,       Color::Rgb(240, 214,  83)),
        "sql"                             => (SQL,       Color::Rgb(255, 160, 122)),
        "har"                             => (NETWORK,   Color::Rgb( 99, 179, 237)),
        "csv"                             => (CSV,       Color::Rgb( 33, 150,  83)),
        "txt"                             => (TEXT,      Color::Rgb(187, 187, 187)),
        "ttf" | "otf" | "woff" | "woff2" => (FONT,      Color::Rgb(200, 160, 255)),
        "pem" | "key" | "crt" | "cert"
        | "p12" | "pfx" | "ca-bundle"    => (KEY,       Color::Rgb(255, 200,  50)),
        "png" | "jpg" | "jpeg" | "gif"
        | "webp" | "bmp" | "tiff" | "ico"
        | "svg"                           => (IMAGE,     Color::Rgb(167, 215,  97)),
        "mp4" | "mov" | "avi" | "mkv"
        | "webm"                          => (VIDEO,     Color::Rgb(253, 199,   0)),
        "mp3" | "wav" | "flac" | "aac"
        | "ogg"                           => (AUDIO,     Color::Rgb(  0, 188, 212)),
        "pdf"                             => (PDF,       Color::Rgb(236,  56,  50)),
        "zip" | "tar" | "gz" | "bz2"
        | "xz" | "7z"                     => (ARCHIVE,   Color::Rgb(240, 214,  83)),
        "lock"                            => (LOCK,      Color::Rgb(183, 183, 183)),
        "o"                               => (BINARY,    Color::Rgb(150, 120,  80)),
        "d"                               => (BINARY,    Color::Rgb(100, 100,  80)),
        "rlib" | "rmeta"                  => (LIB,       Color::Rgb(200, 120,  80)),
        "so" | "dylib" | "dll" | "a"      => (LIB,       Color::Rgb(180, 100,  60)),
        "wasm"                            => (BINARY,    Color::Rgb(100, 150, 200)),
        "pdb" | "map"                     => (BINARY,    Color::Rgb(120, 120, 120)),
        _                                 => (FILE,      Color::Rgb(180, 180, 180)),
    }
}

pub fn icon_for_entry(entry: &Entry) -> (&'static str, Color) {
    if entry.is_dir {
        return (FOLDER, Color::Rgb(97, 175, 239));
    }
    let (icon, color) = icon_for_name(&entry.name);
    if icon == FILE && entry.is_executable {
        return (RUN, Color::Rgb(80, 220, 120));
    }
    (icon, color)
}

pub fn group_label(ext: &str) -> &'static str {
    match ext {
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts"
        | "html" | "htm" | "css" | "scss" | "sass"
        | "py" | "pyi" | "go" | "c" | "h" | "cpp" | "cc" | "cxx" | "hpp"
        | "java" | "swift" | "kt" | "kts" | "rb" | "lua" | "sql" => "Developer",
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" | "conf" | "env"
        | "tf" | "tfvars" | "tfstate" | "gitignore" | "lock"      => "Config",
        "sh" | "bash" | "zsh" | "fish" | "vim" | "nix"            => "Scripts",
        "md" | "mdx" | "txt" | "csv" | "pdf" | "doc" | "docx"
        | "xls" | "xlsx" | "ppt" | "pptx"                         => "Documents",
        "o" | "d" | "rlib" | "rmeta" | "so" | "dylib" | "dll"
        | "a" | "wasm" | "pdb" | "map"                            => "Compiled",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico"
        | "webp" | "bmp" | "tiff"                                  => "Images",
        "mp4" | "mov" | "avi" | "mkv" | "webm"                    => "Video",
        "mp3" | "wav" | "flac" | "aac" | "ogg"                    => "Audio",
        "ttf" | "otf" | "woff" | "woff2"                          => "Fonts",
        "pem" | "key" | "crt" | "cert" | "p12" | "pfx"
        | "ca-bundle"                                              => "Security",
        "har"                                                      => "Network",
        _                                                          => "Other",
    }
}

pub fn read_dir_entries(path: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_string_lossy().into_owned();
            let is_dir = p.is_dir();
            let is_executable = !is_dir
                && p.metadata()
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false);
            Some(Entry { name, path: p, is_dir, is_executable })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}
