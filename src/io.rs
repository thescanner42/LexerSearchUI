use std::collections::BTreeMap;

use base_x::{DecodeError, decode, encode};
use lexer_search_lib::{
    engine::{
        graph::{GraphBuilder, GroupInfo},
        matcher::{FullMatch, Matcher},
        matchers::{make_c_like_lexer, make_python_like_lexer, make_rust_like_lexer},
    },
    io::Language,
    lexer::{
        DEFAULT_MAX_CONCURRENT_MATCHES, DEFAULT_MAX_DISTINCT_GROUPS, DEFAULT_MAX_EXPANSIONS,
        DEFAULT_MAX_GROUP_MEMORY, DEFAULT_MAX_TOKEN_LENGTH, EnumLexer,
    },
};
use serde::{Deserialize, Serialize};

const ALPHABET: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.~/:@!$&()*+,;='";

pub fn encode_bytes(data: &[u8]) -> String {
    encode(ALPHABET, data)
}

pub fn decode_bytes(s: &str) -> Result<Vec<u8>, DecodeError> {
    decode(ALPHABET, s)
}

type Playgroundlhs = Vec<MatchingUnit>;

/// the DTO that is used to serialize and deserialize from the url part
#[derive(Serialize, Deserialize, bincode::Encode, bincode::Decode, Debug)]
pub struct PlaygroundConfig {
    /// the content to scan
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,

    /// the subject's language
    pub language: Language,

    pub lhs: Playgroundlhs,
}

#[derive(Serialize, Deserialize, bincode::Encode, bincode::Decode, Debug)]
pub struct MatchingUnit {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "GroupInfo::is_default")]
    pub group: GroupInfo,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub out: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub transform: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub templates: BTreeMap<String, Vec<String>>,
}

impl Default for PlaygroundConfig {
    fn default() -> Self {
        PlaygroundConfig {
            subject: "// click the \"Docs\" button above!\nhello_world(\"test\");".to_string(),
            language: Language::Rust,
            lhs: vec![MatchingUnit {
                patterns: vec!["hello_world(... $CAPTURE ...)".to_string()],
                name: "hi".to_string(),
                group: Default::default(),
                out: Default::default(),
                transform: Default::default(),
                templates: Default::default(),
            }],
        }
    }
}

pub const PUBLIC_URL: &'static str = include_str!("../target/lexer-search-ui-public-url");

impl PlaygroundConfig {
    pub fn to_url_str(&self) -> String {
        let bin = bincode::encode_to_vec(self, bincode::config::standard()).unwrap();
        let compressed = zstd::encode_all(&bin[..], 22).unwrap();

        encode_bytes(&compressed)
    }

    pub fn from_url_str(mut s: &str) -> Result<Self, String> {
        if s.len() <= PUBLIC_URL.len() {
            return Ok(Default::default());
        }
        if s.starts_with(PUBLIC_URL) {
            s = &s[PUBLIC_URL.len()..];
        }
        let compressed = match decode_bytes(s) {
            Ok(v) => v,
            Err(e) => return Err(e.to_string()),
        };

        let decompressed = match zstd::decode_all(&compressed[..]) {
            Ok(v) => v,
            Err(e) => return Err(e.to_string()),
        };

        let cfg = match bincode::decode_from_slice(&decompressed, bincode::config::standard()) {
            Ok(v) => v,
            Err(e) => return Err(e.to_string()),
        };
        Ok(cfg.0)
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
            Language::C => "cpp",
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

        fn convert_templates(
            input: BTreeMap<String, Vec<String>>,
        ) -> BTreeMap<Box<[u8]>, Vec<Box<[u8]>>> {
            input
                .into_iter()
                .map(|(k, v)| {
                    (
                        k.into_bytes().into_boxed_slice(),
                        v.into_iter()
                            .map(|s| s.into_bytes().into_boxed_slice())
                            .collect(),
                    )
                })
                .collect()
        }

        let mut graph = GraphBuilder::default();
        for unit in self.lhs {
            for unexpanded_pattern in unit.patterns {
                for pattern in lexer_search_lib::engine::template::expand(
                    unexpanded_pattern.as_bytes(),
                    &convert_templates(unit.templates.clone()),
                    DEFAULT_MAX_EXPANSIONS,
                )? {
                    let mut reader = std::io::Cursor::new(pattern);
                    let lexer: EnumLexer = match self.language {
                        Language::C | Language::CSharp | Language::Java => {
                            EnumLexer::CLike(make_c_like_lexer(
                                false,
                                true,
                                DEFAULT_MAX_TOKEN_LENGTH,
                            ))
                        }
                        Language::Go | Language::Js | Language::Ts | Language::Kotlin => {
                            EnumLexer::CLike(make_c_like_lexer(
                                true,
                                true,
                                DEFAULT_MAX_TOKEN_LENGTH,
                            ))
                        }
                        Language::Py => EnumLexer::PythonLike(make_python_like_lexer(
                            true,
                            DEFAULT_MAX_TOKEN_LENGTH,
                        )),
                        Language::Rust => EnumLexer::RustLike(make_rust_like_lexer(
                            true,
                            DEFAULT_MAX_TOKEN_LENGTH,
                        )),
                    };

                    graph.add_pattern(
                        &mut reader,
                        &convert_out(unit.out.clone()),
                        unit.name.clone(),
                        unit.group.clone(),
                        &convert_transform(unit.transform.clone()),
                        lexer,
                        DEFAULT_MAX_TOKEN_LENGTH,
                    )?;
                }
            }
        }

        let graph = graph.build()?;

        let mut matcher = Matcher::new(
            &graph,
            DEFAULT_MAX_CONCURRENT_MATCHES,
            DEFAULT_MAX_TOKEN_LENGTH,
            DEFAULT_MAX_DISTINCT_GROUPS,
            DEFAULT_MAX_GROUP_MEMORY,
            DEFAULT_MAX_EXPANSIONS,
        );

        let mut reader = std::io::Cursor::new(self.subject);
        let lexer: EnumLexer = match self.language {
            Language::C | Language::CSharp | Language::Java => {
                EnumLexer::CLike(make_c_like_lexer(false, false, DEFAULT_MAX_TOKEN_LENGTH))
            }
            Language::Go | Language::Js | Language::Ts | Language::Kotlin => {
                EnumLexer::CLike(make_c_like_lexer(true, false, DEFAULT_MAX_TOKEN_LENGTH))
            }
            Language::Py => {
                EnumLexer::PythonLike(make_python_like_lexer(false, DEFAULT_MAX_TOKEN_LENGTH))
            }
            Language::Rust => {
                EnumLexer::RustLike(make_rust_like_lexer(false, DEFAULT_MAX_TOKEN_LENGTH))
            }
        };

        matcher.process_and_drain(&mut reader, lexer, out)?;

        Ok(())
    }
}
