use std::{collections::BTreeMap, num::NonZeroUsize};

use base_x::{DecodeError, decode, encode};
use lexer_search_lib::{
    engine::{
        matcher::{FullMatch, Matcher, Trie},
        matchers::{make_c_like_lexer, make_python_like_lexer, make_rust_like_lexer},
    },
    io::Language,
    lexer::EnumLexer,
};
use serde::{Deserialize, Serialize};

const ALPHABET: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.~/:@!$&'()*+,;=";

pub fn encode_bytes(data: &[u8]) -> String {
    encode(ALPHABET, data)
}

pub fn decode_bytes(s: &str) -> Result<Vec<u8>, DecodeError> {
    decode(ALPHABET, s)
}

// Default functions
fn default_max_concurrent_matches() -> usize {
    5000
}

fn default_max_token_length() -> NonZeroUsize {
    5000.try_into().unwrap()
}

fn default_group_cap() -> NonZeroUsize {
    NonZeroUsize::new(10).unwrap()
}

type Playgroundlhs = Vec<MatchingUnit>;

/// the DTO that is used to serialize and deserialize from the url part
#[derive(Serialize, Deserialize, Debug)]
pub struct PlaygroundConfig {
    /// the content to scan
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,

    /// the subject's language
    pub language: Language,

    pub lhs: Playgroundlhs,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MatchingUnit {
    pub patterns: Vec<String>,
    pub name: String,
    pub group: String,
    pub out: BTreeMap<String, String>,
    pub transform: BTreeMap<String, String>,
}

impl Default for PlaygroundConfig {
    fn default() -> Self {
        PlaygroundConfig {
            subject: "let x = \"hi\";\nprintln!(\"{x}\");".to_string(),
            language: Language::Rust,
            lhs: vec![MatchingUnit {
                patterns: vec!["&_VAR = $_STR;\n...\nprintln!($_FMT)".to_string()],
                name: "hello_world".to_string(),
                group: "".to_string(),
                out: Default::default(),
                transform: BTreeMap::from([(
                    "_FMT".to_string(),
                    "^\\{(?<_VAR>[^}]+)}$".to_string(),
                )]),
            }],
        }
    }
}

pub const PUBLIC_URL: &'static str = include_str!("../target/lexer-search-ui-public-url");

impl PlaygroundConfig {
    pub fn to_url_str(&self) -> String {
        let bin = bincode::serde::encode_to_vec(self, bincode::config::standard()).unwrap();
        let compressed = zstd::encode_all(&bin[..], 22).unwrap();

        encode_bytes(&compressed)
    }

    pub fn from_url_str(mut s: &str) -> Self {
        if s.starts_with(PUBLIC_URL) {
            s = &s[PUBLIC_URL.len()..];
        }
        let compressed = match decode_bytes(s) {
            Ok(v) => v,
            Err(_) => return Default::default(),
        };

        let decompressed = match zstd::decode_all(&compressed[..]) {
            Ok(v) => v,
            Err(_) => return Default::default(),
        };

        let cfg =
            match bincode::serde::decode_from_slice(&decompressed, bincode::config::standard()) {
                Ok(v) => v,
                Err(_) => return Default::default(),
            };
        cfg.0
    }

    pub fn from_editor_parts(
        subject: &str,
        language: &str,
        editor_lhs: &str,
    ) -> Result<Self, String> {
        let lhs = serde_yml::from_str(editor_lhs).map_err(|e| e.to_string())?;
        let lang = serde_yml::from_str(language).map_err(|e| e.to_string())?;
        Ok(Self {
            subject: subject.to_owned(),
            language: lang,
            lhs: lhs,
        })
    }

    /// lhs, rhs, lang
    pub fn to_editor_parts(self) -> (String, String, String) {
        let lang = self.monaco_language().to_string();
        (self.editor_lhs(), self.subject, lang)
    }

    fn monaco_language(&self) -> &'static str {
        match self.language {
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
            Language::Go => "go",
            Language::Java => "java",
            Language::Js => "javascript",
            Language::Kotlin => "kotlin",
            Language::Py => "python",
            Language::Rust => "rust",
            Language::Ts => "typescript",
        }
    }

    fn editor_lhs(&self) -> String {
        let s = serde_yml::to_string(&self.lhs).unwrap();
        s.to_string()
    }

    pub fn run(self, out: impl FnMut(FullMatch)) -> Result<(), String> {
        fn convert_out(input: BTreeMap<String, String>) -> BTreeMap<Box<[u8]>, Box<[u8]>> {
            input
                .into_iter()
                .map(|(k, v)| {
                    (
                        k.into_bytes().into_boxed_slice(),
                        v.into_bytes().into_boxed_slice(),
                    )
                })
                .collect()
        }

        fn convert_transform(input: BTreeMap<String, String>) -> BTreeMap<Box<[u8]>, String> {
            input
                .into_iter()
                .map(|(k, v)| (k.into_bytes().into_boxed_slice(), v))
                .collect()
        }

        let mut trie = Trie::default();
        for unit in self.lhs {
            for pattern in unit.patterns {
                let mut reader = std::io::Cursor::new(pattern);
                let lexer: EnumLexer = match self.language {
                    Language::C | Language::Cpp | Language::CSharp | Language::Java => {
                        EnumLexer::CLike(make_c_like_lexer(false, true, default_max_token_length()))
                    }
                    Language::Go | Language::Js | Language::Ts | Language::Kotlin => {
                        EnumLexer::CLike(make_c_like_lexer(true, true, default_max_token_length()))
                    }
                    Language::Py => EnumLexer::PythonLike(make_python_like_lexer(
                        true,
                        default_max_token_length(),
                    )),
                    Language::Rust => {
                        EnumLexer::RustLike(make_rust_like_lexer(true, default_max_token_length()))
                    }
                };

                trie.add_pattern(
                    &mut reader,
                    &convert_out(unit.out.clone()),
                    unit.name.clone(),
                    unit.group.clone(),
                    &convert_transform(unit.transform.clone()),
                    lexer,
                    default_max_token_length(),
                )?;
            }
        }

        let mut matcher = Matcher::new(
            &trie,
            default_max_concurrent_matches(),
            default_max_token_length(),
            default_group_cap(),
        );

        let mut reader = std::io::Cursor::new(self.subject);
        let lexer: EnumLexer = match self.language {
            Language::C | Language::Cpp | Language::CSharp | Language::Java => {
                EnumLexer::CLike(make_c_like_lexer(false, false, default_max_token_length()))
            }
            Language::Go | Language::Js | Language::Ts | Language::Kotlin => {
                EnumLexer::CLike(make_c_like_lexer(true, false, default_max_token_length()))
            }
            Language::Py => {
                EnumLexer::PythonLike(make_python_like_lexer(false, default_max_token_length()))
            }
            Language::Rust => {
                EnumLexer::RustLike(make_rust_like_lexer(false, default_max_token_length()))
            }
        };

        matcher.process_and_drain(&mut reader, lexer, out)?;

        Ok(())
    }
}
