pub mod io;

use gloo::events::EventListener;
use monaco::{
    api::CodeEditorOptions,
    sys::editor::BuiltinTheme,
    yew::{CodeEditor, CodeEditorLink},
};
use serde::Serialize;
use serde_json::Value;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::prelude::*;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{MouseEvent, window};
use yew::{Callback, Component, Context, Html, Properties, html};

use crate::io::PlaygroundConfig;

// --------------------
// JS helper function
// --------------------

#[derive(Serialize)]
pub struct HighlightElement {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub class_name: String,
    pub text: Option<String>,
}

#[wasm_bindgen(module = "/src/highlight_helper.js")]
extern "C" {
    fn highlight_ranges_js(editor: &JsValue, elements: &JsValue);
}

// --------------------
// Helpers
// --------------------

fn url_path() -> String {
    let win = window().unwrap();
    let location = win.location();

    // Get the pathname (e.g., "/LexerSearchUI/")
    let pathname = location.pathname().unwrap_or_else(|_| "/".to_string());

    // Get the hash (e.g., "#/test") and include it
    let hash = location.hash().unwrap_or_default();

    // Combine and remove leading slash if present
    let full_path = format!("{}{}", pathname, hash);
    full_path
        .strip_prefix('/')
        .unwrap_or(&full_path)
        .to_string()
}

fn editor_options(content: String, lang: String) -> CodeEditorOptions {
    CodeEditorOptions::default()
        .with_language(lang)
        .with_value(content)
        .with_builtin_theme(BuiltinTheme::VsDark)
        .with_automatic_layout(true)
}

#[derive(Properties, PartialEq)]
struct EditorProps {
    options: Rc<CodeEditorOptions>,
    on_editor_created: Option<Callback<CodeEditorLink>>,
}

struct StableEditor;

impl Component for StableEditor {
    type Message = ();
    type Properties = EditorProps;

    fn create(_: &Context<Self>) -> Self {
        Self
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let on_editor_created = ctx.props().on_editor_created.clone();

        html! {
            <CodeEditor
                classes={"full-height"}
                options={ctx.props().options.to_sys_options()}
                on_editor_created={on_editor_created.unwrap_or_default()}
            />
        }
    }

    fn changed(&mut self, _: &Context<Self>, _: &Self::Properties) -> bool {
        false
    }
}

// --------------------
// Messages
// --------------------
enum Msg {
    StartDrag,
    Drag(i32),
    StopDrag,
    LanguageChanged(String),
    CopyShareLink,
    Run,
}

// --------------------
// App state
// --------------------
struct App {
    left_options: Rc<CodeEditorOptions>,
    right_options: Rc<CodeEditorOptions>,
    left_width: i32,
    mousemove_listener: Option<EventListener>,
    mouseup_listener: Option<EventListener>,
    current_language: String,
    rhs_editor: Rc<RefCell<Option<CodeEditorLink>>>,
    lhs_editor: Rc<RefCell<Option<CodeEditorLink>>>,

    error: Option<String>,
}

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(_ctx: &Context<Self>) -> Self {
        let cfg = PlaygroundConfig::from_url_str(&url_path());
        let (lhs, rhs, lang) = cfg.to_editor_parts();

        Self {
            left_options: Rc::new(editor_options(lhs, "yaml".to_string())),
            right_options: Rc::new(editor_options(rhs, lang.clone())),
            left_width: 500,
            mousemove_listener: None,
            mouseup_listener: None,
            current_language: lang,
            rhs_editor: Rc::new(RefCell::new(None)),
            lhs_editor: Rc::new(RefCell::new(None)),
            error: None,
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            Msg::CopyShareLink | Msg::Run => {
                let was_error = self.error.is_some();
                self.error = None;

                let rhs_content = self
                    .rhs_editor
                    .borrow()
                    .as_ref()
                    .and_then(|editor| editor.with_editor(|m| m.get_model().map(|m| m.get_value())))
                    .unwrap_or_else(|| self.right_options.value.clone())
                    .unwrap_or_default();

                let lhs_content = self
                    .lhs_editor
                    .borrow()
                    .as_ref()
                    .and_then(|editor| editor.with_editor(|m| m.get_model().map(|m| m.get_value())))
                    .unwrap_or_else(|| self.left_options.value.clone())
                    .unwrap_or_default();

                let cfg = match PlaygroundConfig::from_editor_parts(
                    &rhs_content,
                    &self.current_language,
                    &lhs_content,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        // preserve current content
                        self.right_options =
                            Rc::new(editor_options(rhs_content, self.current_language.clone()));
                        self.left_options =
                            Rc::new(editor_options(lhs_content, "yaml".to_string()));
                        self.error = Some(e);
                        return true;
                    }
                };

                match msg {
                    Msg::CopyShareLink => {
                        let path = cfg.to_url_str();
                        let win = web_sys::window().unwrap();
                        let location = win.location();
                        let origin = location.origin().unwrap();
                        let full_url = format!("{}/{}{}", origin, crate::io::PUBLIC_URL, path);
                        let _ = win.navigator().clipboard().write_text(&full_url);
                    }
                    Msg::Run => {
                        let mut accumulate: Vec<HighlightElement> = Default::default();
                        if let Err(e) = cfg.run(|result| {
                            accumulate.push(HighlightElement {
                                start_line: result.start.line,
                                start_col: result.start.column,
                                end_line: result.end.line,
                                end_col: result.end.column,
                                class_name: "match-highlight".to_owned(),
                                text: Some(if !result.captures.is_empty() {
                                    let captures_map: serde_json::Map<String, Value> = result
                                        .captures
                                        .iter()
                                        .map(|(k, v)| {
                                            (
                                                String::from_utf8_lossy(k).to_string(),
                                                Value::String(
                                                    String::from_utf8_lossy(v).to_string(),
                                                ),
                                            )
                                        })
                                        .collect();
                                    let captures_str =
                                        serde_json::to_string(&captures_map).unwrap_or_default();
                                    format!("name: {}", captures_str)
                                } else {
                                    // Just the name
                                    result.name.clone()
                                }),
                            });
                        }) {
                            // preserve current content
                            self.right_options =
                                Rc::new(editor_options(rhs_content, self.current_language.clone()));
                            self.left_options =
                                Rc::new(editor_options(lhs_content, "yaml".to_string()));
                            self.error = Some(e);
                            return true;
                        }

                        if let Some(editor_link) = &*self.rhs_editor.borrow() {
                            editor_link.with_editor(|editor_api: &monaco::api::CodeEditor| {
                                let js_editor: &JsValue = editor_api.as_ref();

                                let js_elements = serde_wasm_bindgen::to_value(&accumulate)
                                    .expect("failed to serialize highlights");
                                highlight_ranges_js(js_editor, &js_elements);
                            });
                        }
                    }
                    _ => unreachable!(),
                }
                was_error
            }
            Msg::StartDrag => {
                let link = ctx.link().clone();
                let win = window().unwrap();

                self.mousemove_listener =
                    Some(EventListener::new(&win, "mousemove", move |event| {
                        let event = event.dyn_ref::<MouseEvent>().unwrap();
                        link.send_message(Msg::Drag(event.client_x()));
                    }));

                let link = ctx.link().clone();
                self.mouseup_listener = Some(EventListener::new(&win, "mouseup", move |_| {
                    link.send_message(Msg::StopDrag);
                }));

                false
            }
            Msg::Drag(x) => {
                self.left_width = x.max(200);

                // Preserve current editor content to prevent clearing during drag
                let lhs_content = self
                    .lhs_editor
                    .borrow()
                    .as_ref()
                    .and_then(|editor| editor.with_editor(|m| m.get_model().map(|m| m.get_value())))
                    .unwrap_or_else(|| self.left_options.value.clone())
                    .unwrap_or_default();

                let rhs_content = self
                    .rhs_editor
                    .borrow()
                    .as_ref()
                    .and_then(|editor| editor.with_editor(|m| m.get_model().map(|m| m.get_value())))
                    .unwrap_or_else(|| self.right_options.value.clone())
                    .unwrap_or_default();

                self.left_options = Rc::new(editor_options(lhs_content, "yaml".to_string()));
                self.right_options =
                    Rc::new(editor_options(rhs_content, self.current_language.clone()));

                true
            }
            Msg::StopDrag => {
                self.mousemove_listener = None;
                self.mouseup_listener = None;
                false
            }
            Msg::LanguageChanged(lang) => {
                self.current_language = lang.clone();

                if let Some(editor) = &*self.rhs_editor.borrow() {
                    editor.with_editor(|e| {
                        if let Some(model) = e.get_model() {
                            model.set_language(&lang);
                        }
                    });
                }

                false
            }
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let total_width = window().unwrap().inner_width().unwrap().as_f64().unwrap() as i32;
        let right_width = (total_width - self.left_width - 6).max(200);

        let on_language_change = ctx.link().callback(|e: web_sys::Event| {
            let select: web_sys::HtmlSelectElement = e.target().unwrap().dyn_into().unwrap();
            Msg::LanguageChanged(select.value())
        });

        let rhs_editor_clone_clone = self.rhs_editor.clone();
        let lhs_editor_clone_clone = self.lhs_editor.clone();

        html! {
            <div style="height:100vh; display:flex; flex-direction:column;">
                // Header
                <div style="
                    height:50px;
                    background:#222;
                    color:white;
                    display:flex;
                    align-items:center;
                    padding:0 10px;
                    gap:10px;
                ">
                    <button onclick={ctx.link().callback(|_| Msg::Run)}>{"Run"}</button>

                    <select onchange={on_language_change}>
                        <option value="c" selected={self.current_language == "c"}>{"C"}</option>
                        <option value="cpp" selected={self.current_language == "cpp"}>{"C++"}</option>
                        <option value="csharp" selected={self.current_language == "csharp"}>{"C#"}</option>
                        <option value="go" selected={self.current_language == "go"}>{"Go"}</option>
                        <option value="java" selected={self.current_language == "java"}>{"Java"}</option>
                        <option value="javascript" selected={self.current_language == "javascript"}>{"JavaScript"}</option>
                        <option value="kotlin" selected={self.current_language == "kotlin"}>{"Kotlin"}</option>
                        <option value="python" selected={self.current_language == "python"}>{"Python"}</option>
                        <option value="rust" selected={self.current_language == "rust"}>{"Rust"}</option>
                        <option value="typescript" selected={self.current_language == "typescript"}>{"TypeScript"}</option>
                    </select>

                    <button onclick={ctx.link().callback(|_| Msg::CopyShareLink)}>{"Copy Share Link"}</button>

                    <button onclick={
                        |_| {
                            if let Some(win) = web_sys::window() {
                                let _ = win.open_with_url_and_target(
                                    "https://github.com/thescanner42/LexerSearch/blob/main/lexer-search-lib/PATTERN-GUIDE.md",
                                    "_blank",
                                );
                            }
                        }
                    }>{"Docs"}</button>
                </div>

                { self.error.as_ref().map(|err| html! {
                    <div style="
                        background:#5a1a1a;
                        color:#ffb3b3;
                        padding:8px;
                        font-family:monospace;
                    ">
                        { format!("Error: {}", err) }
                    </div>
                })}

                // Editors
                <div style="flex:1; display:flex;">
                    <div style={format!("width:{}px;", self.left_width)}>
                        <StableEditor options={self.left_options.clone()}
                            on_editor_created={Some(Callback::from(move |link: CodeEditorLink| {
                                *lhs_editor_clone_clone.borrow_mut() = Some(link);
                            }))} />
                    </div>

                    <div style="width:6px; cursor:col-resize; background:#444;"
                        onmousedown={ctx.link().callback(|_| Msg::StartDrag)} />

                    <div style={format!("width:{}px;", right_width)}>
                        <StableEditor
                            options={self.right_options.clone()}
                            on_editor_created={Some(Callback::from(move |link: CodeEditorLink| {
                                *rhs_editor_clone_clone.borrow_mut() = Some(link);
                            }))}
                        />
                    </div>
                </div>
            </div>
        }
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
