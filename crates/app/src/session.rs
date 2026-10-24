//! The editing session: document, live preview instance, selection and geometry edits.

use std::cell::RefCell;
use std::rc::Rc;

use preview::{Built, Diagnostic, Hit, LiveInstance, compile_with};
use storage::ComponentPreview;
use slint_doc::{
    Document, EditError, ElementInfo, ElementKind, ElementPath, LayoutKind, Op, TreeNode,
};

use crate::anchor::{self, Anchor, Axis, Kind};
use crate::guides::{self, Guides};
use crate::layout_bridge::layout_spec;

const MIN_SIZE: f32 = 4.0;
const TAP_THRESHOLD: f32 = 8.0;

/// Why an axis written as some other expression is not moved, resized or switched.
pub const EXPR_ANCHOR: &str = "式で書かれているため動かせません(付き方を選ぶと動かせます)";
/// Why children anchored right / centre / both sides cannot go into or out of a layout.
const ANCHORED_CHILD: &str =
    "右・下・中央・左右・上下に付いた子要素があるため変えられません(先に左・上に切り替えてください)";

/// The notice when a drag leaves an axis written as an expression where it is.
fn expr_notice(axes: &[Axis]) -> String {
    let names: Vec<&str> =
        axes.iter().map(|a| if *a == Axis::H { "横" } else { "縦" }).collect();
    format!("{} は式で書かれているため動かしません(付き方を選ぶと動かせます)", names.join("・"))
}

fn expr_refusal() -> SessionError {
    SessionError::Anchor(EXPR_ANCHOR.into())
}

/// The anchor of one axis of `info`, from the written `x` / `width` (or `y` / `height`).
fn anchor_of(info: &ElementInfo, axis: Axis) -> Anchor {
    let written = |n: &str| {
        info.bindings.iter().find(|b| b.name == n).map(|b| b.expr.trim()).unwrap_or_default()
    };
    anchor::read(axis, written(axis.pos()), written(axis.size()))
}

/// `(position, size)` of `r` along `axis`.
fn span(r: &tidy::Rect, axis: Axis) -> (f32, f32) {
    match axis {
        Axis::H => (r.x, r.w),
        Axis::V => (r.y, r.h),
    }
}

/// A binding to write: the property and its new expression (`None` removes it).
type Binding = (String, Option<String>);

/// The preview values and the build a text compiled with.
type Compiled = (Vec<(String, String)>, Built);

/// `(old start, old end, new start, new end)` of an edge pair of a resize.
type Edges = (f32, f32, f32, f32);

fn is_start(a: Anchor) -> bool {
    matches!(a, Anchor::Start(_))
}

/// The bindings that write `anchor` on `axis`: the position, then the size when it is part of
/// the anchor (`Stretch`) or `size_px` asks for it as a plain length.
fn anchor_bindings(
    axis: Axis,
    anchor: Anchor,
    size_px: Option<f32>,
) -> Vec<Binding> {
    let Some((pos, size)) = anchor::write(axis, anchor) else { return Vec::new() };
    let mut out = vec![(axis.pos().to_string(), Some(pos))];
    match (size, size_px) {
        (Some(text), _) => out.push((axis.size().to_string(), Some(text))),
        (None, Some(v)) => {
            out.push((axis.size().to_string(), Some(format!("{}px", v.round() as i32))))
        }
        (None, None) => {}
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("要素が選ばれていません")]
    NoSelection,
    #[error("この要素はレイアウトが位置を決めています")]
    InLayout,
    #[error("この要素には子要素を入れられません")]
    NotAContainer,
    #[error("この要素はレイアウトではありません")]
    NotALayout,
    #[error("選んだ要素はレイアウトです。先に「Unstack」で子要素を自由にしてください")]
    TidyOnLayout,
    #[error("選んだ要素は部品の利用ではありません。部品の利用を選んでください")]
    NotAUse,
    /// An anchor edit that cannot keep what is written; nothing was changed.
    #[error("{0}")]
    Anchor(String),
    #[error("プレビューを更新中です。もう一度押してください")]
    PreviewUpdating,
    #[error("整える子要素がありません")]
    NoChildren,
    #[error("プレビューに表示されていない要素です")]
    NoGeometry,
    #[error("コンパイルに失敗しました")]
    Compile(Vec<Diagnostic>),
    /// An edit that would leave the document broken; nothing was changed.
    #[error("この値は使えません: {0}")]
    Rejected(String),
    #[error("{0} は in / in-out プロパティではありません")]
    NotPreviewProp(String),
    #[error("表示後にソースが変わりました。ソースタブを開き直してください")]
    StaleSource,
}

/// Where the component used by the selected element is defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UseTarget {
    /// Declared in the open file.
    Local(String),
    /// Imported: the path as written in the `import`, and the name in that file.
    Imported { path: String, component: String },
}

/// How a property value is entered and normalised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Length,
    Color,
    Text,
    Other,
}

/// One row of the property list; `expr` is empty when the property is not set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropRow {
    pub name: String,
    /// The binding as written in the source; empty when the property is not set.
    pub expr: String,
    /// What the field shows (see `field_text`).
    pub shown: String,
    pub kind: PropKind,
    /// For a parameter of the used component (`in` / `in-out`): its type. Empty for the
    /// element's own properties.
    pub ty: String,
    /// For a parameter: the default expression of the component, as the field would show it
    /// (empty when it has none). Shown faintly while the field is empty.
    pub placeholder: String,
    /// Whether 引数にする is offered: the binding can become a parameter of the component
    /// being edited.
    pub can_param: bool,
}

/// One `in` / `in-out` property of a used component.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UseParam {
    name: String,
    ty: String,
    /// The default expression as written; empty when there is none.
    default: String,
}

/// The default expression of a property declaration (`in property <T> name: expr;`).
fn declaration_default(decl: &str) -> String {
    let Some(i) = decl.find('>') else { return String::new() };
    let rest = decl[i + 1..].trim_start();
    let rest = rest.trim_start_matches(|c: char| c.is_alphanumeric() || c == '-' || c == '_').trim_start();
    match rest.strip_prefix(':') {
        Some(e) => e.trim().trim_end_matches(';').trim().to_string(),
        None => String::new(),
    }
}

pub fn prop_kind(name: &str) -> PropKind {
    match name {
        "x" | "y" | "width" | "height" | "spacing" | "border-radius" | "border-width"
        | "font-size" => PropKind::Length,
        n if n.starts_with("padding") => PropKind::Length,
        "color" | "background" | "border-color" => PropKind::Color,
        "text" => PropKind::Text,
        _ => PropKind::Other,
    }
}

const GEOMETRY_PROPS: [&str; 4] = ["x", "y", "width", "height"];
const LAYOUT_PROPS: [&str; 8] = [
    "spacing",
    "padding",
    "padding-left",
    "padding-right",
    "padding-top",
    "padding-bottom",
    "alignment",
    "cross-axis-alignment",
];
const TEXT_PROPS: [&str; 3] = ["text", "color", "font-size"];
const RECTANGLE_PROPS: [&str; 4] = ["background", "border-radius", "border-width", "border-color"];

/// A bare (optionally negative or decimal) number.
fn is_bare_number(v: &str) -> bool {
    let v = v.strip_prefix('-').unwrap_or(v);
    let mut dots = 0;
    !v.is_empty()
        && v.chars().any(|c| c.is_ascii_digit())
        && v.chars().all(|c| {
            if c == '.' {
                dots += 1;
            }
            c.is_ascii_digit() || c == '.'
        })
        && dots <= 1
}

/// A string literal for `text`: `\` and `"` escaped.
fn string_literal(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// What a property field shows for the binding `expr`. For `text`, a plain string literal is
/// shown as the string itself (no quotes, no escapes; a leading `=` is doubled); any other
/// binding is shown as `=` and the expression. Other properties show the expression.
pub fn field_text(kind: PropKind, expr: &str) -> String {
    if kind != PropKind::Text || expr.is_empty() {
        return expr.to_string();
    }
    match unquote(expr) {
        Some(s) if s.starts_with('=') => format!("={s}"),
        Some(s) => s,
        None => format!("={expr}"),
    }
}

/// The binding expression a field value stands for. For `text`: `=expr` is the expression
/// `expr`, `==abc` is the text `=abc`, anything else is a plain string that is quoted and
/// escaped here, so nothing typed can end the literal. `Err` is a reason to show.
fn normalise(kind: PropKind, value: &str) -> Result<String, String> {
    Ok(match kind {
        PropKind::Length if is_bare_number(value) => format!("{value}px"),
        PropKind::Text => match value.strip_prefix('=') {
            Some(rest) if rest.starts_with('=') => string_literal(rest),
            Some(rest) if rest.trim().is_empty() => {
                return Err("= のあとに式を書いてください".into());
            }
            Some(rest) => rest.trim().to_string(),
            None => string_literal(value),
        },
        _ => value.to_string(),
    })
}

/// How a preview value is entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Bool,
    Int,
    Float,
    Length,
    Text,
    /// Anything else: the Slint expression as text.
    Raw,
}

impl PreviewKind {
    fn of(ty: &str) -> Self {
        match ty {
            "bool" => Self::Bool,
            "int" => Self::Int,
            "float" => Self::Float,
            "length" => Self::Length,
            "string" => Self::Text,
            _ => Self::Raw,
        }
    }
}

/// One `in` / `in-out` property of the edited component, with its preview value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewProp {
    pub name: String,
    pub ty: String,
    pub kind: PreviewKind,
    /// What the field shows: for a string, a plain literal as the text itself and any other
    /// expression as `=` and the expression (see `field_text`); otherwise the Slint
    /// expression. Empty when no value is set.
    pub shown: String,
    /// Why the value was left out of the last build; empty when it applies.
    pub error: String,
}

/// Screen sizes offered for the canvas (width, height in logical px).
pub const SCREEN_SIZES: [(u32, u32); 4] = [(360, 640), (800, 360), (914, 411), (1280, 720)];
pub const DEFAULT_SCREEN: (u32, u32) = SCREEN_SIZES[0];

/// `800x360`, `800×360`, `800 * 360` -> (800, 360).
pub fn parse_screen_size(text: &str) -> Option<(u32, u32)> {
    let t = text.trim();
    let (w, h) = t.split_once(['x', 'X', '×', '*'])?;
    let (w, h) = (w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?);
    ((1..=10_000).contains(&w) && (1..=10_000).contains(&h)).then_some((w, h))
}

/// `in property <T> name ...` / `in-out property <T> name ...` -> `(name, T)`.
fn public_property(decl: &str) -> Option<(String, String)> {
    let rest = decl.trim();
    let rest = rest.strip_prefix("in-out").or_else(|| rest.strip_prefix("in"))?;
    let rest = rest.trim_start().strip_prefix("property")?.trim_start().strip_prefix('<')?;
    let mut depth = 1;
    let mut end = None;
    for (i, c) in rest.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    let ty = rest[..end].trim().to_string();
    let name: String = rest[end + 1..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!name.is_empty()).then_some((name, ty))
}

/// `"abc"` -> `abc` when the expression is exactly one plain string literal.
fn unquote(expr: &str) -> Option<String> {
    let inner = expr.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return None,
            '\\' => match it.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                _ => return None,
            },
            c => out.push(c),
        }
    }
    Some(out)
}

/// The Slint expression a preview field input stands for (the `.preview` file keeps the
/// expression, so old files keep working). `string` follows the Props `text` rule (`normalise`);
/// a raw value takes an optional leading `=`; the other kinds keep their input. Empty clears.
fn preview_expr(kind: PreviewKind, input: &str) -> Result<String, String> {
    let v = input.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    Ok(match kind {
        PreviewKind::Length if is_bare_number(v) => format!("{v}px"),
        PreviewKind::Text => normalise(PropKind::Text, input)?,
        PreviewKind::Raw => match v.strip_prefix('=') {
            Some(rest) if rest.trim().is_empty() => {
                return Err("= のあとに式を書いてください".into());
            }
            Some(rest) => rest.trim().to_string(),
            None => v.to_string(),
        },
        _ => v.to_string(),
    })
}

/// One row of the element tree, in depth-first display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub path: ElementPath,
    pub depth: usize,
    /// `"id := Type"` or `"Type"`.
    pub label: String,
    pub is_layout: bool,
}

/// Operations of a tree row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeAction {
    /// Reorder within the same parent.
    Up,
    Down,
    /// Become the next sibling of the parent.
    Out,
    /// Become the last child of the previous sibling, which must be a container.
    In,
    Delete,
    /// Append a new `Rectangle` to the element's parent.
    NewContainer,
}

/// Standard palette entries, in display order.
pub const STD_PALETTE: [&str; 13] = [
    "Rectangle",
    "Text",
    "Image",
    "TouchArea",
    "HorizontalLayout",
    "VerticalLayout",
    "Button",
    "CheckBox",
    "Slider",
    "SpinBox",
    "LineEdit",
    "ComboBox",
    "Switch",
];

/// Palette entries that live in `std-widgets.slint` and need an import.
const STD_WIDGETS: [&str; 7] = [
    "Button", "CheckBox", "Slider", "SpinBox", "LineEdit", "ComboBox", "Switch",
];

/// Assumed size of a freshly inserted element (its real size is unknown before compiling).
const NEW_SIZE: f32 = 40.0;
const PLACE_STEP: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The kinds of declaration the extract dialog can add to a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    InProperty,
    InOutProperty,
    OutProperty,
    Callback,
}

/// Types offered for a declared property.
pub const DECL_TYPES: [&str; 8] = [
    "string", "int", "float", "bool", "length", "color", "brush", "image",
];

impl DeclKind {
    pub fn from_index(i: i32) -> Option<Self> {
        match i {
            0 => Some(Self::InProperty),
            1 => Some(Self::InOutProperty),
            2 => Some(Self::OutProperty),
            3 => Some(Self::Callback),
            _ => None,
        }
    }
}

/// The type of a property that takes part in "make this a parameter", from its name.
pub fn parameter_type(property: &str) -> Option<&'static str> {
    match property {
        "text" => Some("string"),
        "x" | "y" | "width" | "height" | "font-size" | "border-radius" | "border-width"
        | "spacing" => Some("length"),
        p if p.starts_with("padding") => Some("length"),
        "color" | "background" | "border-color" => Some("brush"),
        "visible" | "enabled" => Some("bool"),
        "opacity" => Some("float"),
        "source" => Some("image"),
        _ => None,
    }
}

/// `[A-Za-z_][A-Za-z0-9_-]*`
pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The one-line declaration, e.g. `in property <string> label;` or `callback clicked();`.
/// `ty` is ignored for callbacks.
pub fn declaration_text(kind: DeclKind, ty: &str, name: &str) -> String {
    match kind {
        DeclKind::InProperty => format!("in property <{ty}> {name};"),
        DeclKind::InOutProperty => format!("in-out property <{ty}> {name};"),
        DeclKind::OutProperty => format!("out property <{ty}> {name};"),
        DeclKind::Callback => format!("callback {name}();"),
    }
}

/// What a free Tidy did: the relations it found (member indices refer to `children`) and how
/// many children were rewritten.
#[derive(Debug, Clone)]
pub struct TidyReport {
    pub relations: Vec<tidy::free::Relation>,
    pub changed: usize,
    pub container: ElementPath,
    /// Path of every child, in the order the relations index them.
    pub children: Vec<ElementPath>,
    /// Children whose axes written as expressions were left alone (for the toast).
    pub notice: Option<String>,
}

impl TidyReport {
    /// One line for the toast, e.g. `そろえ 3・等間隔 1`; kinds that did not occur are omitted.
    pub fn summary(&self) -> String {
        use tidy::free::Relation as R;
        if self.changed == 0 {
            return "整える箇所はありませんでした".into();
        }
        let count = |f: fn(&R) -> bool| self.relations.iter().filter(|r| f(r)).count();
        let parts = [
            ("そろえ", count(|r| matches!(r, R::Align { .. }))),
            ("等間隔", count(|r| matches!(r, R::EqualGap { .. }))),
            ("対称", count(|r| matches!(r, R::Symmetric { .. }))),
            ("繰り返し", count(|r| matches!(r, R::Repeat { .. }))),
            ("寸法", count(|r| matches!(r, R::SameSize { .. }))),
        ];
        let text: Vec<String> =
            parts.iter().filter(|(_, n)| *n > 0).map(|(k, n)| format!("{k} {n}")).collect();
        if text.is_empty() { format!("{} 件を整えました", self.changed) } else { text.join("・") }
    }
}

/// Callers must call `rebuild()` after each successful edit: geometry (tap, selection, move,
/// resize) comes from the last successful build, resolved against `snapshot`, the text that
/// build compiled.
pub struct Session {
    doc: Document,
    /// The text the live instance was built from. Replaced only when the instance is.
    snapshot: Rc<RefCell<Document>>,
    on_instance: Option<Rc<dyn Fn()>>,
    component: String,
    live: Rc<RefCell<Option<LiveInstance>>>,
    selection: Option<ElementPath>,
    last_tap: Option<(f32, f32)>,
    pub grid: i32,
    diagnostics: Vec<Diagnostic>,
    /// Preview values and screen size per component of this file.
    previews: std::collections::BTreeMap<String, ComponentPreview>,
    /// `(property, reason)` of the values the last successful build left out.
    dropped: Vec<(String, String)>,
    /// The compile of the text a property edit just produced; `rebuild` uses it instead of
    /// compiling the same text again.
    prepared: Option<Prepared>,
    /// What the last move left alone, for the toast (see `take_notice`).
    notice: Option<String>,
}

struct Prepared {
    text: String,
    path: std::path::PathBuf,
    component: String,
    values: Vec<(String, String)>,
    built: Built,
}

fn first_message(d: &[Diagnostic]) -> String {
    d.first().map(|d| d.message.clone()).unwrap_or_default()
}

/// A diagnostic cut to a length that fits next to a field.
fn short(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(60).collect::<String>())
    } else {
        line.to_string()
    }
}

/// Tail of the error band while the document does not compile: the ways out.
pub const DIAGNOSTIC_HINT: &str = "(元に戻す か ソース で直せます)";

fn snap(v: f32, grid: i32) -> i32 {
    if grid <= 0 {
        return v.round() as i32;
    }
    let g = grid as f32;
    ((v / g).round() * g) as i32
}

impl Session {
    pub fn new(doc: Document) -> Self {
        let comps = doc.components();
        let component = comps
            .iter()
            .find(|c| c.exported)
            .or_else(|| comps.first())
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let snapshot = Document::new(doc.path().to_path_buf(), doc.text().to_string());
        Session {
            snapshot: Rc::new(RefCell::new(snapshot)),
            on_instance: None,
            doc,
            component,
            live: Rc::new(RefCell::new(None)),
            selection: None,
            last_tap: None,
            grid: 4,
            diagnostics: Vec::new(),
            previews: Default::default(),
            dropped: Vec::new(),
            prepared: None,
            notice: None,
        }
    }

    /// A note about the last move or resize (an axis left alone because it is an expression),
    /// once.
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// The stored previews of this file's components (before the first `rebuild`).
    pub fn set_previews(&mut self, previews: std::collections::BTreeMap<String, ComponentPreview>) {
        self.previews = previews;
    }

    /// Preview values and size of the current component.
    pub fn preview(&self) -> ComponentPreview {
        self.previews.get(&self.component).cloned().unwrap_or_default()
    }

    /// Screen size of the current component's canvas.
    pub fn screen_size(&self) -> (u32, u32) {
        self.preview().size.unwrap_or(DEFAULT_SCREEN)
    }

    pub fn set_screen_size(&mut self, size: (u32, u32)) {
        self.previews.entry(self.component.clone()).or_default().size =
            (size != DEFAULT_SCREEN).then_some(size);
    }

    /// The `in` / `in-out` properties of the current component with their preview values and
    /// the reason of any value that was left out of the last build.
    pub fn preview_props(&self) -> Vec<PreviewProp> {
        let p = self.preview();
        let Some(info) = self.doc.components().into_iter().find(|c| c.name == self.component) else {
            return Vec::new();
        };
        info.declarations
            .iter()
            .filter_map(|d| public_property(d))
            .map(|(name, ty)| {
                let kind = PreviewKind::of(&ty);
                let expr = p.value(&name).unwrap_or_default();
                let shown = if kind == PreviewKind::Text {
                    field_text(PropKind::Text, expr)
                } else {
                    expr.to_string()
                };
                let error = self
                    .dropped
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_default();
                PreviewProp { name, ty, kind, shown, error }
            })
            .collect()
    }

    /// The stored values that name a declared `in` / `in-out` property of the current
    /// component: only those are ever compiled (a stale or hand-edited `.preview` entry is not).
    fn effective_values(&self) -> Vec<(String, String)> {
        let names: Vec<String> = self.preview_props().into_iter().map(|p| p.name).collect();
        self.preview().values.into_iter().filter(|(n, _)| names.contains(n)).collect()
    }

    /// Texts the live preview shows right now (see `LiveInstance::visible_texts`).
    pub fn visible_texts(&self) -> Vec<String> {
        self.live.borrow().as_ref().map(|l| l.visible_texts()).unwrap_or_default()
    }

    /// Sets (an empty input removes) the preview value of a property of the current
    /// component; `rebuild` applies it. `NotPreviewProp` when it is not an `in` / `in-out`
    /// property, `Rejected` (nothing changed) for a lone `=`.
    pub fn set_preview_value(&mut self, name: &str, input: &str) -> Result<(), SessionError> {
        let Some(prop) = self.preview_props().into_iter().find(|p| p.name == name) else {
            return Err(SessionError::NotPreviewProp(name.to_string()));
        };
        let expr = preview_expr(prop.kind, input).map_err(SessionError::Rejected)?;
        self.previews.entry(self.component.clone()).or_default().set_value(name, &expr);
        Ok(())
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn component(&self) -> &str {
        &self.component
    }

    pub fn set_component(&mut self, name: &str) {
        self.component = name.to_string();
        self.selection = None;
        self.last_tap = None;
    }

    pub fn selection(&self) -> Option<&ElementPath> {
        self.selection.as_ref()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn mark_saved(&mut self) {
        self.doc.mark_saved();
    }

    /// The document now lives at `path` (rename / save as); call `rebuild()` afterwards.
    pub fn set_path(&mut self, path: std::path::PathBuf) {
        self.doc.set_path(path);
    }

    /// Called whenever a factory from `rebuild()` has created its instance, i.e. when the
    /// geometry queries start answering for the new build.
    pub fn set_on_instance(&mut self, f: impl Fn() + 'static) {
        self.on_instance = Some(Rc::new(f));
    }

    /// Compiles the current text with the preview values, or takes the compile `set_property`
    /// already made of exactly this text.
    fn build(&mut self) -> Result<Built, Vec<Diagnostic>> {
        let values = self.effective_values();
        if let Some(p) = self.prepared.take()
            && p.text == self.doc.text()
            && p.path == self.doc.path()
            && p.component == self.component
            && p.values == values
        {
            return Ok(p.built);
        }
        compile_with(self.doc.text(), self.doc.path(), &self.component, &values)
    }

    /// The error band text for the current diagnostics: the first message and the ways out;
    /// empty when the document compiles.
    pub fn diagnostic_banner(&self) -> String {
        match self.diagnostics.first() {
            Some(d) => format!("{} {DIAGNOSTIC_HINT}", d.message),
            None => String::new(),
        }
    }

    fn snapshot_doc(&self) -> Document {
        Document::new(self.doc.path().to_path_buf(), self.doc.text().to_string())
    }

    /// Compiles the document; on success returns a factory. The new snapshot is installed
    /// together with the instance, when the container runs the factory, so geometry queries
    /// always see an instance and the text it was built from.
    pub fn rebuild(&mut self) -> Option<slint::ComponentFactory> {
        match self.build() {
            Ok(b) => {
                let c = b.compiled;
                self.dropped = b.dropped;
                self.diagnostics.clear();
                let pending = self.snapshot_doc();
                let snapshot = self.snapshot.clone();
                let hook = self.on_instance.clone();
                Some(c.factory_with(self.live.clone(), move || {
                    *snapshot.borrow_mut() = Document::new(
                        pending.path().to_path_buf(),
                        pending.text().to_string(),
                    );
                    if let Some(h) = &hook {
                        h();
                    }
                }))
            }
            Err(d) => {
                self.diagnostics = d;
                self.dropped.clear();
                None
            }
        }
    }

    /// Compiles and instantiates standalone (for tests).
    pub fn rebuild_standalone(&mut self, width: f32, height: f32) -> bool {
        match self.build() {
            Ok(b) => {
                let c = b.compiled;
                self.dropped = b.dropped;
                self.diagnostics.clear();
                *self.snapshot.borrow_mut() = self.snapshot_doc();
                *self.live.borrow_mut() = Some(c.instantiate(width, height));
                true
            }
            Err(d) => {
                self.diagnostics = d;
                self.dropped.clear();
                false
            }
        }
    }

    /// `(path, type, id)` of every element of the current component, in tree order.
    fn shape(doc: &Document, component: &str) -> Vec<(ElementPath, String, Option<String>)> {
        fn walk(n: &TreeNode, out: &mut Vec<(ElementPath, String, Option<String>)>) {
            out.push((n.info.path.clone(), n.info.type_name.clone(), n.info.id.clone()));
            for c in &n.children {
                walk(c, out);
            }
        }
        let mut out = Vec::new();
        if let Some(t) = doc.tree(component) {
            walk(&t, &mut out);
        }
        out
    }

    /// False when the preview is stale (the source does not compile) and the current
    /// component's element tree differs from the one the preview was built from, so paths from
    /// the preview would name other elements in the document.
    fn preview_matches_doc(&self) -> bool {
        if self.diagnostics.is_empty() {
            return true;
        }
        let snapshot = self.snapshot.borrow();
        snapshot.text() == self.doc.text()
            || Self::shape(&snapshot, &self.component) == Self::shape(&self.doc, &self.component)
    }

    fn refuse_if_mismatch(&self) -> Result<(), SessionError> {
        if self.preview_matches_doc() {
            Ok(())
        } else {
            Err(SessionError::Compile(self.diagnostics.clone()))
        }
    }

    pub fn tap(&mut self, x: f32, y: f32) -> Option<ElementPath> {
        if !self.preview_matches_doc() {
            self.selection = None;
            self.last_tap = None;
            return None;
        }
        let hits = self.live.borrow().as_ref()?.hits_at(x, y);
        let snapshot = self.snapshot.borrow();
        let mut candidates: Vec<ElementPath> = hits
            .iter()
            .filter_map(|h| snapshot.path_at_offset(h.start))
            .filter(|p| p.component == self.component)
            .collect();
        // The component root is the last ancestor; hit testing itself never reports it.
        let root = ElementPath { component: self.component.clone(), indices: vec![] };
        if !candidates.is_empty() && !candidates.contains(&root) {
            candidates.push(root);
        }
        let near = self.last_tap.is_some_and(|(lx, ly)| {
            (lx - x).abs() <= TAP_THRESHOLD && (ly - y).abs() <= TAP_THRESHOLD
        });
        self.last_tap = Some((x, y));
        let current = self
            .selection
            .as_ref()
            .and_then(|s| candidates.iter().position(|c| c == s));
        let pick = match current {
            Some(i) if near => (i + 1) % candidates.len(),
            _ => 0,
        };
        self.selection = candidates.get(pick).cloned();
        self.selection.clone()
    }

    /// The definition of the component the selected element uses, when it is a use of a
    /// user-defined component (in this file or imported).
    pub fn selected_use(&self) -> Option<UseTarget> {
        let info = self.doc.element(self.selection.as_ref()?)?;
        let name = info.type_name;
        if self.doc.components().iter().any(|c| c.name == name) {
            return Some(UseTarget::Local(name));
        }
        self.doc.imports().into_iter().find_map(|i| {
            // Only files of the project can be opened (not std-widgets or subfolders).
            let file = i.path.strip_prefix("./").unwrap_or(&i.path);
            if !file.ends_with(".slint") || file.contains('/') || file == "std-widgets.slint" {
                return None;
            }
            let (external, _) = i.names.into_iter().find(|(_, internal)| *internal == name)?;
            Some(UseTarget::Imported { path: i.path, component: external })
        })
    }

    pub fn selection_hit(&self) -> Option<Hit> {
        self.hit_of(self.selection.as_ref()?)
    }

    /// Whether the selection can be moved or resized on the canvas (the component root cannot).
    pub fn selection_movable(&self) -> bool {
        self.selection.as_ref().is_some_and(|p| !p.indices.is_empty())
    }

    /// The selected path and its geometry, refusing the root and layout children.
    fn movable(&self) -> Result<(ElementPath, Hit, bool), SessionError> {
        let path = self.selection.clone().ok_or(SessionError::NoSelection)?;
        if path.indices.is_empty() {
            return Err(EditError::InvalidTarget(
                "ルート要素は移動も大きさ変更もできません".into(),
            )
            .into());
        }
        self.refuse_if_mismatch()?;
        let mut parent = path.clone();
        parent.indices.pop();
        let parent_info = self
            .doc
            .element(&parent)
            .ok_or_else(|| EditError::ElementNotFound(parent.clone()))?;
        let in_layout = matches!(parent_info.kind, ElementKind::Layout(_));
        let hit = self.selection_hit().ok_or(SessionError::NoGeometry)?;
        Ok((path, hit, in_layout))
    }

    /// The horizontal and vertical anchor of the selection, read from its `x` / `width` and
    /// `y` / `height`. `None` for the component root and without a selection. Inside a layout
    /// these are only what is written (a layout decides the position; see `can_set_anchor`).
    pub fn anchors(&self) -> Option<(Anchor, Anchor)> {
        let path = self.selection.as_ref()?;
        if path.indices.is_empty() {
            return None;
        }
        let info = self.doc.element(path)?;
        Some((anchor_of(&info, Axis::H), anchor_of(&info, Axis::V)))
    }

    /// Whether anchors apply to the selection: a free-placed child whose container is in the
    /// preview.
    pub fn can_set_anchor(&self) -> bool {
        self.movable().is_ok_and(|(path, _, in_layout)| {
            !in_layout && self.parent_length(&path, Axis::H).is_some()
        })
    }

    /// The size of the container of `path` along `axis`: what `parent.width` / `parent.height`
    /// is for it, from the preview.
    fn parent_length(&self, path: &ElementPath, axis: Axis) -> Option<f32> {
        let mut parent = path.clone();
        parent.indices.pop();
        Some(span(&self.hit_of(&parent)?.local, axis).1)
    }

    fn need_parent_length(&self, path: &ElementPath, axis: Axis) -> Result<f64, SessionError> {
        self.parent_length(path, axis).map(f64::from).ok_or(SessionError::NoGeometry)
    }

    /// Switches the anchor of one axis of the selection to `kind`, keeping the element where
    /// it is on screen (and as large) in the current preview; one history step. Refused in a
    /// layout. An axis written as some other expression is replaced: choosing an anchor is the
    /// way out of it.
    pub fn set_anchor(&mut self, axis: Axis, kind: Kind) -> Result<(), SessionError> {
        let (path, hit, in_layout) = self.movable()?;
        if in_layout {
            return Err(SessionError::InLayout);
        }
        let info = self
            .doc
            .element(&path)
            .ok_or_else(|| EditError::ElementNotFound(path.clone()))?;
        let current = anchor_of(&info, axis);
        if current.kind() == Some(kind) {
            return Ok(());
        }
        let (pos, size) = span(&hit.local, axis);
        let parent = self.need_parent_length(&path, axis)?;
        let anchor = anchor::solve(kind, f64::from(pos), f64::from(size), parent);
        // Leaving `Stretch`, or an expression, the size that was an expression becomes the
        // length it has now.
        let written_size = info
            .bindings
            .iter()
            .find(|b| b.name == axis.size())
            .map(|b| b.expr.trim())
            .unwrap_or_default();
        let odd_size = !written_size.is_empty() && anchor::plain_length(written_size).is_none();
        let size_px = (matches!(current, Anchor::Stretch(..)) || odd_size).then_some(size);
        let bindings = anchor_bindings(axis, anchor, size_px);
        self.apply_validated(Op::SetBindings { path, bindings })
    }

    pub fn move_selection(&mut self, dx: f32, dy: f32) -> Result<(), SessionError> {
        let (path, hit, in_layout) = self.movable()?;
        if in_layout {
            return Err(SessionError::InLayout);
        }
        let info = self
            .doc
            .element(&path)
            .ok_or_else(|| EditError::ElementNotFound(path.clone()))?;
        let axes = [(Axis::H, dx), (Axis::V, dy)].map(|(a, d)| (a, anchor_of(&info, a), d));
        if !axes.iter().all(|&(_, a, _)| is_start(a)) {
            return self.move_anchored(path, &hit, &axes);
        }
        let x = snap(hit.local.x + dx, self.grid);
        let y = snap(hit.local.y + dy, self.grid);
        self.apply(Op::SetBindings {
            path,
            bindings: vec![
                ("x".into(), Some(format!("{x}px"))),
                ("y".into(), Some(format!("{y}px"))),
            ],
        })
    }

    /// A move with at least one axis not anchored at the start: only axes that were dragged
    /// are written, each keeping its anchor with its stored numbers moved by the (grid-snapped)
    /// delta. An axis written as an expression stays and gets a notice (`take_notice`).
    fn move_anchored(
        &mut self,
        path: ElementPath,
        hit: &Hit,
        axes: &[(Axis, Anchor, f32); 2],
    ) -> Result<(), SessionError> {
        self.notice = None;
        if axes.iter().all(|&(_, a, _)| a == Anchor::Expr) {
            return Err(expr_refusal());
        }
        let (mut bindings, mut skipped) = (Vec::new(), Vec::new());
        for &(axis, anchor, d) in axes {
            if d == 0.0 {
                continue;
            }
            match anchor {
                Anchor::Expr => skipped.push(axis),
                Anchor::Start(_) => {
                    let new = snap(span(&hit.local, axis).0 + d, self.grid);
                    bindings.push((axis.pos().to_string(), Some(format!("{new}px"))));
                }
                other => {
                    let delta = f64::from(snap(d, self.grid));
                    let moved = match other {
                        Anchor::End(b) => Anchor::End(b - delta),
                        Anchor::Center(c) => Anchor::Center(c + delta),
                        Anchor::Stretch(a, b) => Anchor::Stretch(a + delta, b - delta),
                        _ => continue,
                    };
                    if delta != 0.0 {
                        bindings.extend(anchor_bindings(axis, moved, None));
                    }
                }
            }
        }
        if !skipped.is_empty() {
            self.notice = Some(expr_notice(&skipped));
        }
        if bindings.is_empty() {
            return Ok(());
        }
        self.apply_validated(Op::SetBindings { path, bindings })
    }

    pub fn resize_selection(
        &mut self,
        corner: Corner,
        dx: f32,
        dy: f32,
    ) -> Result<(), SessionError> {
        let (path, hit, in_layout) = self.movable()?;
        let r = hit.local;
        let (left, top, right, bottom) = (r.x, r.y, r.x + r.w, r.y + r.h);
        let g = self.grid;
        let (mut l, mut t, mut rt, mut b) = (left, top, right, bottom);
        let (moves_left, moves_top) = match corner {
            Corner::TopLeft => (true, true),
            Corner::TopRight => (false, true),
            Corner::BottomLeft => (true, false),
            Corner::BottomRight => (false, false),
        };
        if moves_left {
            l = (snap(left + dx, g) as f32).min(right - MIN_SIZE);
        } else {
            rt = (snap(right + dx, g) as f32).max(left + MIN_SIZE);
        }
        if moves_top {
            t = (snap(top + dy, g) as f32).min(bottom - MIN_SIZE);
        } else {
            b = (snap(bottom + dy, g) as f32).max(top + MIN_SIZE);
        }
        let info = self
            .doc
            .element(&path)
            .ok_or_else(|| EditError::ElementNotFound(path.clone()))?;
        let axes = [(Axis::H, (left, right, l, rt)), (Axis::V, (top, bottom, t, b))]
            .map(|(a, edges)| (a, anchor_of(&info, a), edges));
        if !in_layout && !axes.iter().all(|&(_, a, _)| is_start(a)) {
            self.notice = None;
            return self.resize_anchored(path, &axes);
        }
        let mut bindings = Vec::new();
        if !in_layout {
            bindings.push(("x".into(), Some(format!("{}px", l as i32))));
            bindings.push(("y".into(), Some(format!("{}px", t as i32))));
        }
        bindings.push((
            "width".into(),
            Some(format!("{}px", (rt - l).round() as i32)),
        ));
        bindings.push((
            "height".into(),
            Some(format!("{}px", (b - t).round() as i32)),
        ));
        self.apply(Op::SetBindings { path, bindings })
    }

    /// A resize with at least one axis not anchored at the start. Edges are
    /// `(old start, old end, new start, new end)`; an axis keeps its anchor, and only the numbers
    /// of the edges that moved change.
    fn resize_anchored(
        &mut self,
        path: ElementPath,
        axes: &[(Axis, Anchor, Edges); 2],
    ) -> Result<(), SessionError> {
        let same = |a: f32, b: f32| (a - b).abs() < 0.001;
        let (mut positions, mut sizes) = (Vec::new(), Vec::new());
        for &(axis, anchor, (old_a, old_b, a, b)) in axes {
            let unchanged = same(old_a, a) && same(old_b, b);
            let size = b - a;
            match anchor {
                Anchor::Expr if unchanged => {}
                Anchor::Expr => return Err(expr_refusal()),
                Anchor::Start(_) => {
                    positions.push((axis.pos().to_string(), Some(format!("{}px", a as i32))));
                    sizes.push((axis.size().to_string(), Some(format!("{}px", size.round() as i32))));
                }
                _ if unchanged => {}
                other => {
                    let parent = self.need_parent_length(&path, axis)?;
                    let kind = other.kind().expect("not an expression");
                    let solved = anchor::solve(kind, f64::from(a), f64::from(size), parent);
                    let mut written = anchor_bindings(axis, solved, Some(size)).into_iter();
                    positions.extend(written.next());
                    sizes.extend(written);
                }
            }
        }
        positions.extend(sizes);
        self.apply_validated(Op::SetBindings { path, bindings: positions })
    }

    /// Applies an edit. While the preview is stale (`diagnostics` non-empty) structural ops
    /// (insert, remove, move, wrap, unwrap, extract) are refused with `Compile`; `SetBindings`
    /// and `AddDeclaration` stay allowed.
    pub fn apply(&mut self, op: Op) -> Result<(), SessionError> {
        let structural = !matches!(
            op,
            Op::SetBindings { .. } | Op::AddDeclaration { .. } | Op::Parameterize { .. }
        );
        if structural && !self.diagnostics.is_empty() && !self.removal_is_safe(&op) {
            return Err(SessionError::Compile(self.diagnostics.clone()));
        }
        if let Some(p) = self.doc.apply(op)? {
            self.selection = Some(p);
        }
        Ok(())
    }

    /// Deleting is the way out of a document that does not compile, so it stays possible then:
    /// the path comes from the document's own tree (not from the stale preview) and the edit
    /// must not add a syntax error.
    fn removal_is_safe(&self, op: &Op) -> bool {
        matches!(op, Op::RemoveElement { .. })
            && self
                .doc
                .preview_apply(op.clone())
                .is_ok_and(|t| self.refuse_new_syntax_errors(&t).is_ok())
    }

    /// Moves the selected element into a new component `name`; the selection becomes the
    /// usage that replaced it.
    pub fn extract_selection(&mut self, name: &str) -> Result<(), SessionError> {
        let path = self.selection.clone().ok_or(SessionError::NoSelection)?;
        self.apply(Op::ExtractComponent { path, name: name.to_string() })
    }

    /// Why `name` cannot name a new component, if it cannot.
    pub fn component_name_error(&self, name: &str) -> Option<String> {
        if !is_identifier(name) {
            return Some("名前に使えるのは英数字・_・- だけです(先頭は英字か _)".into());
        }
        if self.doc.components().iter().any(|c| c.name == name)
            || self.doc.global_names().iter().any(|g| g == name)
        {
            return Some("同じ名前の部品があります".into());
        }
        None
    }

    /// Adds a property or callback declaration to `component`.
    pub fn add_declaration(
        &mut self,
        component: &str,
        kind: DeclKind,
        ty: &str,
        name: &str,
    ) -> Result<(), SessionError> {
        if !is_identifier(name) {
            return Err(EditError::InvalidTarget(format!("`{name}` は名前に使えません")).into());
        }
        if kind != DeclKind::Callback && !DECL_TYPES.contains(&ty) {
            return Err(EditError::InvalidTarget(format!("`{ty}` は使えない型です")).into());
        }
        self.apply(Op::AddDeclaration {
            component: component.to_string(),
            text: declaration_text(kind, ty, name),
        })
    }

    fn revalidate(&mut self) {
        if let Some(s) = &self.selection
            && self.doc.element(s).is_none()
        {
            self.selection = None;
        }
    }

    /// The `in` / `in-out` properties of the component the selection uses, in declaration order.
    /// Empty when the selection is not a use, or its definition cannot be found or read (a
    /// broken import): no error is shown then.
    fn use_params(&self) -> Vec<UseParam> {
        let Some(target) = self.selected_use() else { return Vec::new() };
        let info = match target {
            UseTarget::Local(name) => self.doc.components().into_iter().find(|c| c.name == name),
            UseTarget::Imported { path, component } => {
                let file = match self.doc.path().parent() {
                    Some(dir) => dir.join(path.strip_prefix("./").unwrap_or(&path)),
                    None => return Vec::new(),
                };
                std::fs::read_to_string(&file).ok().and_then(|text| {
                    Document::new(file, text).components().into_iter().find(|c| c.name == component)
                })
            }
        };
        info.map(|c| {
            c.declarations
                .iter()
                .filter_map(|d| {
                    public_property(d).map(|(name, ty)| UseParam {
                        name,
                        ty,
                        default: declaration_default(d),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// Property rows of the selection: geometry first, then (for a use of a component) the
    /// component's `in` / `in-out` properties, then the ones that suit its kind, then any
    /// other written binding. Empty without a selection.
    pub fn properties(&self) -> Vec<PropRow> {
        self.property_rows(true)
    }

    fn property_rows(&self, flags: bool) -> Vec<PropRow> {
        let Some(info) = self.selection.as_ref().and_then(|s| self.doc.element(s)) else {
            return Vec::new();
        };
        let params: Vec<UseParam> = self
            .use_params()
            .into_iter()
            .filter(|p| !GEOMETRY_PROPS.contains(&p.name.as_str()))
            .collect();
        let mut names: Vec<&str> = GEOMETRY_PROPS.to_vec();
        if matches!(info.kind, ElementKind::Layout(_)) {
            names.extend(LAYOUT_PROPS);
        }
        match info.type_name.as_str() {
            "Text" => names.extend(TEXT_PROPS),
            "Rectangle" => names.extend(RECTANGLE_PROPS),
            _ => {}
        }
        let written = |n: &str| {
            info.bindings.iter().find(|b| b.name == n).map(|b| b.expr.clone()).unwrap_or_default()
        };
        let can_param = |n: &str, expr: &str, ty: Option<&str>| {
            flags
                && !expr.is_empty()
                && ty.is_none_or(|t| DECL_TYPES.contains(&t))
                && self.can_parameterize_as(n, ty).is_ok()
        };
        let plain = |n: &str| {
            let expr = written(n);
            PropRow {
                name: n.to_string(),
                shown: field_text(prop_kind(n), &expr),
                kind: prop_kind(n),
                ty: String::new(),
                placeholder: String::new(),
                can_param: can_param(n, &expr, None),
                expr,
            }
        };
        let param_row = |p: &UseParam| {
            let kind = match p.ty.as_str() {
                "string" => PropKind::Text,
                "length" => PropKind::Length,
                _ => PropKind::Other,
            };
            let expr = written(&p.name);
            PropRow {
                name: p.name.clone(),
                shown: field_text(kind, &expr),
                kind,
                ty: p.ty.clone(),
                placeholder: field_text(kind, &p.default),
                can_param: can_param(&p.name, &expr, Some(&p.ty)),
                expr,
            }
        };
        let mut rows: Vec<PropRow> = Vec::new();
        for (i, n) in names.iter().enumerate() {
            if i == GEOMETRY_PROPS.len() {
                rows.extend(params.iter().filter(|p| !names.contains(&p.name.as_str())).map(param_row));
            }
            rows.push(plain(n));
        }
        if names.len() == GEOMETRY_PROPS.len() {
            rows.extend(params.iter().filter(|p| !names.contains(&p.name.as_str())).map(param_row));
        }
        for b in &info.bindings {
            if !names.contains(&b.name.as_str()) && !params.iter().any(|p| p.name == b.name) {
                rows.push(plain(&b.name));
            }
        }
        rows
    }

    /// Sets (or, for an empty value, removes) a property of the selection.
    ///
    /// The edit is made on a scratch copy of the text first. It is refused with `Rejected`,
    /// leaving the document, history and preview untouched, when it would add a syntax error
    /// or, while the document compiles, make it stop compiling. A compile that succeeded is
    /// kept and reused by the next `rebuild`.
    pub fn set_property(&mut self, name: &str, value: &str) -> Result<(), SessionError> {
        let path = self.selection.clone().ok_or(SessionError::NoSelection)?;
        self.refuse_if_mismatch()?;
        // Only the emptiness check and non-text values are trimmed: `"Hi "` keeps its space.
        let rows = self.property_rows(false);
        let row = rows.iter().find(|r| r.name == name);
        let kind = row.map_or_else(|| prop_kind(name), |r| r.kind);
        let value = if kind == PropKind::Text && !value.trim().is_empty() {
            value
        } else {
            value.trim()
        };
        let expr = if value.is_empty() {
            None
        } else {
            Some(normalise(kind, value).map_err(SessionError::Rejected)?)
        };
        let current = row.map(|r| r.expr.clone()).unwrap_or_default();
        if current == expr.clone().unwrap_or_default() {
            return Ok(());
        }
        if let Some(bindings) = self.anchored_geometry(&path, name, expr.as_deref())? {
            return self.apply_validated(Op::SetBindings { path, bindings });
        }
        self.apply_validated(Op::SetBindings { path, bindings: vec![(name.to_string(), expr)] })
    }

    /// The bindings for a length typed into `x` / `y` / `width` / `height` of an element whose
    /// axis is anchored at the end, centre or both sides: the typed number is the position (or
    /// size) it gets and the anchor stays, only its numbers change. `None` when the usual
    /// write applies: another property, a layout child, an axis anchored at the start, a
    /// removal, or an expression typed on purpose. A number on an axis written as some other
    /// expression, or a size on a stretched axis, is refused.
    fn anchored_geometry(
        &self,
        path: &ElementPath,
        name: &str,
        expr: Option<&str>,
    ) -> Result<Option<Vec<Binding>>, SessionError> {
        let axis = match name {
            "x" | "width" => Axis::H,
            "y" | "height" => Axis::V,
            _ => return Ok(None),
        };
        let Some(typed) = expr.and_then(anchor::plain_length) else { return Ok(None) };
        if path.indices.is_empty() {
            return Ok(None);
        }
        let mut parent = path.clone();
        parent.indices.pop();
        if self.doc.element(&parent).is_none_or(|p| matches!(p.kind, ElementKind::Layout(_))) {
            return Ok(None);
        }
        let info = self
            .doc
            .element(path)
            .ok_or_else(|| EditError::ElementNotFound(path.clone()))?;
        let anchor = anchor_of(&info, axis);
        let Some(kind) = anchor.kind() else {
            // Only the property that holds the expression (and would be overwritten) is
            // refused; a plain length next to an expression is written as always.
            let written = info
                .bindings
                .iter()
                .find(|b| b.name == name)
                .map(|b| b.expr.trim())
                .unwrap_or_default();
            let overwrites = !written.is_empty() && anchor::plain_length(written).is_none();
            return if overwrites { Err(expr_refusal()) } else { Ok(None) };
        };
        if kind == Kind::Start {
            return Ok(None);
        }
        let is_size = name == axis.size();
        if is_size && kind == Kind::Stretch {
            let what = match axis {
                Axis::H => "左右に付いているため幅は数では決められません",
                Axis::V => "上下に付いているため高さは数では決められません",
            };
            return Err(SessionError::Anchor(what.into()));
        }
        if is_size {
            // End and centre keep B / C: only the length changes.
            return Ok(Some(vec![(name.to_string(), expr.map(str::to_string))]));
        }
        let hit = self.selection_hit().ok_or(SessionError::NoGeometry)?;
        let size = span(&hit.local, axis).1;
        let parent = self.need_parent_length(path, axis)?;
        let solved = anchor::solve(kind, typed, f64::from(size), parent);
        Ok(Some(anchor_bindings(axis, solved, None)))
    }

    /// Applies `op` after trying it on a scratch copy: refused with `Rejected` (nothing
    /// changed) when it adds a syntax error or, while the document compiles, breaks the
    /// compile. A compile that succeeded is kept for the next `rebuild`.
    fn apply_validated(&mut self, op: Op) -> Result<(), SessionError> {
        let new_text = self.doc.preview_apply(op.clone())?;
        let built = self.check_text(&new_text)?;
        self.apply(op)?;
        debug_assert_eq!(self.doc.text(), new_text);
        self.keep_compile(new_text, built);
        Ok(())
    }

    /// Refuses `new_text` (nothing changed) when it adds a syntax error or, while the document
    /// compiles, stops compiling. The compile that succeeded is returned for `keep_compile`.
    fn check_text(
        &self,
        new_text: &str,
    ) -> Result<Option<Compiled>, SessionError> {
        self.refuse_new_syntax_errors(new_text)?;
        // A document that already does not compile is not made worse by one more binding;
        // the user may be on the way to fixing it.
        if !self.diagnostics.is_empty() {
            return Ok(None);
        }
        let values = self.effective_values();
        match compile_with(new_text, self.doc.path(), &self.component, &values) {
            Ok(b) => Ok(Some((values, b))),
            Err(d) => Err(SessionError::Rejected(short(&first_message(&d)))),
        }
    }

    /// Keeps the compile of `text` (now the document's text) for the next `rebuild`.
    fn keep_compile(&mut self, text: String, built: Option<Compiled>) {
        if let Some((values, b)) = built {
            self.prepared = Some(Prepared {
                text,
                path: self.doc.path().to_path_buf(),
                component: self.component.clone(),
                values,
                built: b,
            });
        }
    }

    /// The type a parameter made from `property` gets, from the name alone.
    pub fn parameter_type(property: &str) -> Option<&'static str> {
        parameter_type(property)
    }

    /// Whether `property` of the selection can become a parameter of the component being
    /// edited: the default name and the type the table gives (`None`: the user must choose),
    /// or the reason (short Japanese text) it cannot.
    pub fn can_parameterize(
        &self,
        property: &str,
    ) -> Result<(String, Option<&'static str>), String> {
        self.can_parameterize_as(property, self.declared_type(property))
    }

    /// The type of `property` of the selection: the one the used component declares for it,
    /// else the one the name table gives. `None`: the user must choose (also for a declared
    /// type that a parameter cannot have).
    fn declared_type(&self, property: &str) -> Option<&'static str> {
        match self.use_params().into_iter().find(|p| p.name == property) {
            Some(p) => DECL_TYPES.iter().find(|t| **t == p.ty).copied(),
            None => parameter_type(property),
        }
    }

    fn can_parameterize_as(
        &self,
        property: &str,
        ty: Option<&str>,
    ) -> Result<(String, Option<&'static str>), String> {
        let ty = ty.and_then(|t| DECL_TYPES.iter().find(|d| **d == t).copied());
        let path = self.selection.as_ref().ok_or("要素が選ばれていません")?;
        if path.component != self.component {
            return Err("部品の中を開いてから選んでください".into());
        }
        self.doc
            .parameter_source(path, property)
            .map_err(|e| match e {
                EditError::InvalidTarget(m) => m,
                e => e.to_string(),
            })?;
        let info = self.doc.element(path).ok_or("要素が見つかりません")?;
        let base = match &info.id {
            Some(id) => id.clone(),
            None => info.type_name.rsplit('.').next().unwrap_or("").to_lowercase(),
        };
        let first = format!("{base}-{property}");
        let mut name = first.clone();
        let mut n = 2;
        while self.doc.parameter_name_taken(&path.component, &name) {
            name = format!("{first}-{n}");
            n += 1;
        }
        Ok((name, ty))
    }

    /// Why `name` cannot be the name of a new parameter of the component being edited.
    pub fn parameter_name_error(&self, name: &str) -> Option<String> {
        if !is_identifier(name) {
            Some("名前に使えるのは英数字・_・- だけです(先頭は英字か _)".to_string())
        } else if self.doc.parameter_name_taken(&self.component, name) {
            Some(format!("{name} はすでに使われています"))
        } else {
            None
        }
    }

    /// Turns `property` of the selection into a parameter `name` of the component, in one
    /// history step. `ty` overrides the table; without either the type is unknown and it is
    /// refused. Refusals leave the document untouched.
    pub fn parameterize(
        &mut self,
        property: &str,
        name: &str,
        ty: Option<&str>,
    ) -> Result<(), SessionError> {
        self.can_parameterize(property).map_err(SessionError::Rejected)?;
        let path = self.selection.clone().ok_or(SessionError::NoSelection)?;
        self.refuse_if_mismatch()?;
        if !is_identifier(name) {
            return Err(EditError::InvalidTarget(format!("`{name}` は名前に使えません")).into());
        }
        let ty = match ty {
            Some(t) if DECL_TYPES.contains(&t) => t,
            Some(t) => {
                return Err(EditError::InvalidTarget(format!("`{t}` は使えない型です")).into());
            }
            None => self
                .declared_type(property)
                .ok_or_else(|| SessionError::Rejected("型が分かりません。型を選んでください".into()))?,
        };
        self.apply_validated(Op::Parameterize {
            path,
            property: property.to_string(),
            name: name.to_string(),
            ty: ty.to_string(),
        })
    }

    /// `Rejected` when `new_text` has a syntax error the current text does not have.
    fn refuse_new_syntax_errors(&self, new_text: &str) -> Result<(), SessionError> {
        match slint_doc::new_syntax_errors(self.doc.text(), new_text, self.doc.path()).first() {
            Some(e) => Err(SessionError::Rejected(short(e))),
            None => Ok(()),
        }
    }

    /// Byte range of the source the Source tab shows: the selected element, else the whole
    /// current component.
    pub fn source_range(&self) -> Option<std::ops::Range<usize>> {
        match &self.selection {
            Some(s) => self.doc.element_range(s),
            None => self.doc.component_range(&self.component),
        }
    }

    /// Byte range of the whole component containing `offset`; the whole file when `offset`
    /// is `None` or lies in no component.
    pub fn component_range_at(&self, offset: Option<u32>) -> std::ops::Range<usize> {
        let whole = 0..self.doc.text().len();
        let Some(offset) = offset else { return whole };
        let offset = offset as usize;
        self.doc
            .components()
            .into_iter()
            .filter_map(|c| self.doc.component_range(&c.name))
            .find(|r| r.contains(&offset) || r.end == offset)
            .unwrap_or(whole)
    }

    /// Replaces only `start..start + original.len()` of the document with `new`, provided the
    /// text there is still `original`; otherwise `StaleSource` and nothing changes.
    pub fn replace_source(
        &mut self,
        start: usize,
        original: &str,
        new: &str,
    ) -> Result<(), SessionError> {
        let text = self.doc.text();
        let end = start + original.len();
        if text.get(start..end) != Some(original) {
            return Err(SessionError::StaleSource);
        }
        let updated = format!("{}{}{}", &text[..start], new, &text[end..]);
        self.replace_text(updated);
        Ok(())
    }

    pub fn replace_text(&mut self, text: String) {
        self.doc.replace_text(text);
        self.revalidate();
    }

    pub fn undo(&mut self) -> bool {
        let r = self.doc.undo();
        self.revalidate();
        r
    }

    pub fn redo(&mut self) -> bool {
        let r = self.doc.redo();
        self.revalidate();
        r
    }

    pub fn select_path(&mut self, path: ElementPath) {
        if self.doc.element(&path).is_some() {
            self.selection = Some(path);
        }
    }

    pub fn tree_rows(&self) -> Vec<TreeRow> {
        fn walk(n: &TreeNode, depth: usize, out: &mut Vec<TreeRow>) {
            let label = match &n.info.id {
                Some(id) => format!("{id} := {}", n.info.type_name),
                None => n.info.type_name.clone(),
            };
            out.push(TreeRow {
                path: n.info.path.clone(),
                depth,
                label,
                is_layout: matches!(n.info.kind, ElementKind::Layout(_)),
            });
            for c in &n.children {
                walk(c, depth + 1, out);
            }
        }
        let mut out = Vec::new();
        if let Some(t) = self.doc.tree(&self.component) {
            walk(&t, 0, &mut out);
        }
        out
    }

    /// The standard parts followed by the other components of the document.
    pub fn palette(&self) -> Vec<String> {
        let mut v: Vec<String> = STD_PALETTE.iter().map(|s| s.to_string()).collect();
        v.extend(
            self.doc
                .components()
                .into_iter()
                .map(|c| c.name)
                .filter(|n| *n != self.component),
        );
        v
    }

    /// A container (palette insertion goes inside it) is a component root, a `Rectangle`,
    /// `TouchArea`, `Window` or any layout; every other element is a leaf.
    fn is_container(info: &ElementInfo) -> bool {
        info.path.indices.is_empty()
            || matches!(info.kind, ElementKind::Layout(_))
            || matches!(info.type_name.as_str(), "Rectangle" | "TouchArea" | "Window")
    }

    fn local_rect_of(&self, path: &ElementPath) -> Option<(f32, f32, f32, f32)> {
        let hit = self.hit_of(path)?;
        Some((hit.local.x, hit.local.y, hit.local.w, hit.local.h))
    }

    /// First 8px-grid position (x first, then y) where an assumed 40x40 element fits inside
    /// `parent` without overlapping or touching its existing children.
    ///
    /// `hint` is the top-left of the visible design area; the scan starts there (converted to
    /// the parent's coordinates, clamped inside it) and wraps to the container's start. A full
    /// container cascades: +8px from its last child.
    fn free_position(
        &self,
        parent: &ElementPath,
        child_count: usize,
        hint: Option<(f32, f32)>,
    ) -> (i32, i32) {
        let origin = (PLACE_STEP as i32, PLACE_STEP as i32);
        let Some((_, _, pw, ph)) = self.local_rect_of(parent) else {
            return origin;
        };
        let (mut sx, mut sy) = (PLACE_STEP, PLACE_STEP);
        if let (Some((hx, hy)), Some(h)) = (hint, self.hit_of(parent)) {
            let snap8 = |v: f32| (v / PLACE_STEP).round() * PLACE_STEP;
            sx = snap8(hx - h.rect.x).clamp(PLACE_STEP, (pw - NEW_SIZE).max(PLACE_STEP));
            sy = snap8(hy - h.rect.y).clamp(PLACE_STEP, (ph - NEW_SIZE).max(PLACE_STEP));
        }
        let kids: Vec<_> = (0..child_count)
            .filter_map(|i| {
                let mut p = parent.clone();
                p.indices.push(i);
                self.local_rect_of(&p)
            })
            .collect();
        let free = |x: f32, y: f32| {
            !kids.iter().any(|&(kx, ky, kw, kh)| {
                x <= kx + kw && kx <= x + NEW_SIZE && y <= ky + kh && ky <= y + NEW_SIZE
            })
        };
        // First the part from the hint onwards, then the whole container from its start.
        for (from_x, from_y) in [(sx, sy), (PLACE_STEP, PLACE_STEP)] {
            let mut y = from_y;
            while y + NEW_SIZE <= ph {
                let mut x = if y == from_y { from_x } else { PLACE_STEP };
                while x + NEW_SIZE <= pw {
                    if free(x, y) {
                        return (x as i32, y as i32);
                    }
                    x += PLACE_STEP;
                }
                y += PLACE_STEP;
            }
        }
        match kids.last() {
            Some(&(kx, ky, ..)) => {
                ((kx + PLACE_STEP).max(PLACE_STEP) as i32, (ky + PLACE_STEP).max(PLACE_STEP) as i32)
            }
            None => origin,
        }
    }

    fn refuse_if_stale(&self) -> Result<(), SessionError> {
        if self.diagnostics.is_empty() {
            Ok(())
        } else {
            Err(SessionError::Compile(self.diagnostics.clone()))
        }
    }

    /// Inserts `name` at the end of `parent`, adding the `std-widgets.slint` import when needed,
    /// as a single history step. Selects the new element.
    fn insert_part(
        &mut self,
        parent: ElementPath,
        name: &str,
        hint: Option<(f32, f32)>,
    ) -> Result<(), SessionError> {
        self.refuse_if_stale()?;
        let parent_info = self
            .doc
            .element(&parent)
            .ok_or_else(|| EditError::ElementNotFound(parent.clone()))?;
        let mut props = Vec::new();
        if !matches!(parent_info.kind, ElementKind::Layout(_)) {
            let (x, y) = self.free_position(&parent, parent_info.child_count, hint);
            props.push(format!("x: {x}px;"));
            props.push(format!("y: {y}px;"));
        }
        match name {
            "Rectangle" | "TouchArea" => {
                props.push("width: 40px;".into());
                props.push("height: 40px;".into());
                if name == "Rectangle" {
                    props.push("background: #808890;".into());
                }
            }
            "Text" => props.push("text: \"Text\";".into()),
            "Button" => props.push("text: \"Button\";".into()),
            "CheckBox" => props.push("text: \"CheckBox\";".into()),
            "Switch" => props.push("text: \"Switch\";".into()),
            _ => {}
        }
        let source = if props.is_empty() {
            format!("{name} {{ }}")
        } else {
            format!("{name} {{ {} }}", props.join(" "))
        };
        // Edit a scratch copy so the insert and the import land in one history step.
        let mut scratch = Document::new(self.doc.path().to_path_buf(), self.doc.text().to_string());
        let new_path = scratch
            .apply(Op::InsertElement { parent, index: None, source })?
            .ok_or_else(|| EditError::InvalidTarget("挿入した要素が見つかりません".into()))?;
        let mut text = scratch.text().to_string();
        if STD_WIDGETS.contains(&name) {
            text = with_std_import(&text, name);
        }
        self.doc.replace_text(text);
        self.selection = Some(new_path);
        Ok(())
    }

    /// Adds a part: into the selection when it is a container, else into its parent (the
    /// component root when nothing is selected). The receiving container is selected afterwards.
    ///
    /// `hint` is the top-left of the visible design area, where the part is preferably placed.
    pub fn add_from_palette(
        &mut self,
        name: &str,
        hint: Option<(f32, f32)>,
    ) -> Result<(), SessionError> {
        self.refuse_if_stale()?;
        let target = match &self.selection {
            None => ElementPath { component: self.component.clone(), indices: vec![] },
            Some(sel) => {
                let info = self
                    .doc
                    .element(sel)
                    .ok_or_else(|| EditError::ElementNotFound(sel.clone()))?;
                if Self::is_container(&info) {
                    sel.clone()
                } else {
                    let mut p = sel.clone();
                    p.indices.pop();
                    p
                }
            }
        };
        self.insert_part(target.clone(), name, hint)?;
        // Keep the receiving container selected: repeated adds become siblings instead of
        // nesting into the part just added (Rectangle counts as a container).
        self.selection = Some(target);
        Ok(())
    }

    fn child_path(parent: &ElementPath, i: usize) -> ElementPath {
        let mut p = parent.clone();
        p.indices.push(i);
        p
    }

    /// Geometry in the live preview of the element at `path`.
    fn hit_of(&self, path: &ElementPath) -> Option<Hit> {
        if path.indices.is_empty() {
            return self.live.borrow().as_ref()?.root_geometry();
        }
        let info = self.snapshot.borrow().element(path)?;
        self.live.borrow().as_ref()?.geometry(info.start)
    }

    /// Wrapping into a layout drops the children's `x` / `y`, and unwrapping writes them
    /// again, so an anchor other than left / top would be lost: refused. Unwrapping also
    /// refuses an `x` / `y` expression it would overwrite; wrapping drops those as ever.
    fn refuse_anchored_children(
        &self,
        parent: &ElementPath,
        count: usize,
        unwrap: bool,
    ) -> Result<(), SessionError> {
        let anchored = (0..count).any(|i| {
            self.doc.element(&Self::child_path(parent, i)).is_some_and(|c| {
                // Unwrapping writes `x` / `y` as px over whatever is there.
                let overwritten = unwrap
                    && c.bindings.iter().any(|b| {
                        matches!(b.name.as_str(), "x" | "y")
                            && anchor::plain_length(b.expr.trim()).is_none()
                    });
                overwritten
                    || [Axis::H, Axis::V].iter().any(|&a| {
                        matches!(
                            anchor_of(&c, a),
                            Anchor::End(_) | Anchor::Center(_) | Anchor::Stretch(..)
                        )
                    })
            })
        });
        if anchored { Err(SessionError::Anchor(ANCHORED_CHILD.into())) } else { Ok(()) }
    }

    /// Wraps the children of the selected container in an inferred layout and selects it.
    /// The positions come from the live preview; nothing changes when any is missing.
    pub fn stack_selection(&mut self) -> Result<Vec<tidy::Warning>, SessionError> {
        let path = self.selection.clone().ok_or(SessionError::NoSelection)?;
        self.refuse_if_mismatch()?;
        let info = self
            .doc
            .element(&path)
            .ok_or_else(|| EditError::ElementNotFound(path.clone()))?;
        if !Self::is_container(&info) || matches!(info.kind, ElementKind::Layout(_)) {
            return Err(SessionError::NotAContainer);
        }
        for i in 0..info.child_count {
            let kind = self.doc.element(&Self::child_path(&path, i)).map(|c| c.kind);
            if matches!(kind, Some(ElementKind::Repeated | ElementKind::Conditional)) {
                return Err(EditError::InvalidTarget(
                    "for / if の子を持つ入れ物は並べ直せません".into(),
                )
                .into());
            }
        }
        self.refuse_anchored_children(&path, info.child_count, false)?;
        let container = self.hit_of(&path).ok_or(SessionError::NoGeometry)?;
        let children = (0..info.child_count)
            .map(|i| self.hit_of(&Self::child_path(&path, i)).map(|h| h.local))
            .collect::<Option<Vec<_>>>()
            .ok_or(SessionError::NoGeometry)?;
        let plan = tidy::plan(container.rect.w, container.rect.h, &children, self.grid)
            .ok_or(SessionError::NotAContainer)?;
        self.apply(Op::WrapInLayout { container: path, layout: layout_spec(&plan) })?;
        Ok(plan.warnings)
    }

    /// Nudges the direct children of the selected container (free placement, no layout) onto
    /// intended values; see `tidy::free`. Rewrites `x` / `y` and explicit `width` / `height`
    /// as integer `px`, all as one history step. The selection stays on the container.
    pub fn tidy_free_selection(&mut self, tolerance: f32) -> Result<TidyReport, SessionError> {
        let path = self.selection.clone().ok_or(SessionError::NoSelection)?;
        self.refuse_if_stale()?;
        if self.snapshot.borrow().text() != self.doc.text() {
            return Err(SessionError::PreviewUpdating);
        }
        let info = self
            .doc
            .element(&path)
            .ok_or_else(|| EditError::ElementNotFound(path.clone()))?;
        if matches!(info.kind, ElementKind::Layout(_)) {
            return Err(SessionError::TidyOnLayout);
        }
        if !Self::is_container(&info) {
            return Err(SessionError::NotAContainer);
        }
        let mut infos = Vec::new();
        for i in 0..info.child_count {
            let ci = self
                .doc
                .element(&Self::child_path(&path, i))
                .ok_or_else(|| EditError::ElementNotFound(Self::child_path(&path, i)))?;
            if matches!(ci.kind, ElementKind::Repeated | ElementKind::Conditional) {
                return Err(EditError::InvalidTarget(
                    "for / if の子を持つ入れ物は整えられません".into(),
                )
                .into());
            }
            infos.push(ci);
        }
        if infos.is_empty() {
            return Err(SessionError::NoChildren);
        }
        let container = self.hit_of(&path).ok_or(SessionError::NoGeometry)?;
        let children = infos
            .iter()
            .map(|ci| {
                let local = self.hit_of(&ci.path)?.local;
                let has = |n: &str| ci.bindings.iter().any(|b| b.name == n);
                Some(tidy::free::Child {
                    rect: local,
                    w_explicit: has("width"),
                    h_explicit: has("height"),
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(SessionError::NoGeometry)?;
        let out = tidy::free::tidy(container.local.w, container.local.h, &children, tolerance);

        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        let mut scratch = self.snapshot_doc();
        let mut changed = 0;
        let mut skipped: Vec<ElementPath> = Vec::new();
        let mut anchored_write = false;
        for ((ci, old), new) in infos.iter().zip(&children).zip(&out.rects) {
            let mut bindings = Vec::new();
            let current = |n: &str| ci.bindings.iter().find(|b| b.name == n).map(|b| b.expr.trim());
            let anchors = [Axis::H, Axis::V].map(|a| (a, anchor_of(ci, a)));
            if anchors.iter().all(|&(_, a)| is_start(a)) {
                let mut set = |name: &str, old: f32, new: f32| {
                    let text = format!("{}px", new.round() as i32);
                    // Only a different written value counts as a change.
                    if !near(old, new) && current(name) != Some(text.as_str()) {
                        bindings.push((name.to_string(), Some(text)));
                    }
                };
                set("x", old.rect.x, new.x);
                set("y", old.rect.y, new.y);
                if old.w_explicit {
                    set("width", old.rect.w, new.w);
                }
                if old.h_explicit {
                    set("height", old.rect.h, new.h);
                }
            } else {
                let explicit = [old.w_explicit, old.h_explicit];
                let parent_len = [container.local.w, container.local.h];
                let (mut positions, mut sizes) = (Vec::new(), Vec::new());
                for (i, (axis, anchor)) in anchors.into_iter().enumerate() {
                    let ((old_pos, old_size), (new_pos, new_size)) =
                        (span(&old.rect, axis), span(new, axis));
                    // A length that is not written is not changed.
                    let new_size = if explicit[i] { new_size } else { old_size };
                    if near(old_pos, new_pos) && near(old_size, new_size) {
                        continue;
                    }
                    let push = |into: &mut Vec<(String, Option<String>)>, name: &str, text: String| {
                        if current(name) != Some(text.as_str()) {
                            into.push((name.to_string(), Some(text)));
                        }
                    };
                    match anchor {
                        Anchor::Expr => skipped.push(ci.path.clone()),
                        Anchor::Start(_) => {
                            if !near(old_pos, new_pos) {
                                push(&mut positions, axis.pos(), format!("{}px", new_pos.round() as i32));
                            }
                            if explicit[i] && !near(old_size, new_size) {
                                push(&mut sizes, axis.size(), format!("{}px", new_size.round() as i32));
                            }
                        }
                        other => {
                            let kind = other.kind().expect("not an expression");
                            let solved = anchor::solve(
                                kind,
                                f64::from(new_pos),
                                f64::from(new_size),
                                f64::from(parent_len[i]),
                            );
                            let (pos_text, size_text) =
                                anchor::write(axis, solved).expect("not an expression");
                            push(&mut positions, axis.pos(), pos_text);
                            match size_text {
                                Some(t) => push(&mut sizes, axis.size(), t),
                                None if explicit[i] && !near(old_size, new_size) => push(
                                    &mut sizes,
                                    axis.size(),
                                    format!("{}px", new_size.round() as i32),
                                ),
                                None => {}
                            }
                        }
                    }
                }
                bindings = positions;
                bindings.extend(sizes);
                anchored_write |= !bindings.is_empty();
            }
            if bindings.is_empty() {
                continue;
            }
            changed += 1;
            scratch.apply(Op::SetBindings { path: ci.path.clone(), bindings })?;
        }
        if changed > 0 {
            let text = scratch.text().to_string();
            // Anchored writes are checked like any property edit.
            let built = if anchored_write { self.check_text(&text)? } else { None };
            self.doc.replace_text(text.clone());
            self.keep_compile(text, built);
        }
        skipped.dedup();
        let notice = (!skipped.is_empty()).then(|| {
            let names: Vec<String> = skipped
                .iter()
                .map(|p| {
                    let ty = self.doc.element(p).map(|i| i.type_name).unwrap_or_default();
                    format!("{ty} #{}", p.indices.last().map_or(0, |i| i + 1))
                })
                .collect();
            format!("式で書かれた軸は動かしませんでした: {}", names.join("、"))
        });
        Ok(TidyReport {
            notice,
            relations: out.relations,
            changed,
            container: path,
            children: infos.into_iter().map(|c| c.path).collect(),
        })
    }

    /// Absolute (design area) origin of `container` and rectangles of `children`, from the
    /// current preview. `None` when any is missing.
    pub fn tidy_geometry(
        &self,
        container: &ElementPath,
        children: &[ElementPath],
    ) -> Option<((f32, f32), Vec<tidy::Rect>)> {
        let c = self.hit_of(container)?.rect;
        let rects = children
            .iter()
            .map(|p| self.hit_of(p).map(|h| h.rect))
            .collect::<Option<Vec<_>>>()?;
        Some(((c.x, c.y), rects))
    }

    /// The one layout that guides and untidy act on: the selection when it is a horizontal or
    /// vertical layout, else the selected container's only horizontal/vertical layout child
    /// (other children may exist). A container with several such layouts is ambiguous: `None`.
    fn guide_layout(&self) -> Option<(ElementPath, ElementInfo, LayoutKind)> {
        if !self.preview_matches_doc() {
            return None;
        }
        let path = self.selection.clone()?;
        let info = self.doc.element(&path)?;
        let stack = |i: &ElementInfo| match i.kind {
            ElementKind::Layout(k @ (LayoutKind::Horizontal | LayoutKind::Vertical)) => Some(k),
            _ => None,
        };
        if let Some(k) = stack(&info) {
            return Some((path, info, k));
        }
        if !Self::is_container(&info) {
            return None;
        }
        let mut found = (0..info.child_count).filter_map(|i| {
            let p = Self::child_path(&path, i);
            let ci = self.doc.element(&p)?;
            stack(&ci).map(|k| (p, ci, k))
        });
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Path of the layout the guides currently describe.
    pub fn guide_layout_path(&self) -> Option<ElementPath> {
        self.guide_layout().map(|(p, ..)| p)
    }

    /// Replaces the layout found by the same rule as the guides by its children, keeping
    /// their on-screen positions rounded to the grid. `NotALayout` when there is none.
    pub fn unstack_selection(&mut self) -> Result<(), SessionError> {
        self.selection.as_ref().ok_or(SessionError::NoSelection)?;
        self.refuse_if_mismatch()?;
        let (layout, layout_info, _) = self.guide_layout().ok_or(SessionError::NotALayout)?;
        let mut parent = layout.clone();
        parent.indices.pop();
        self.refuse_anchored_children(&layout, layout_info.child_count, true)?;
        let origin = self.hit_of(&parent).ok_or(SessionError::NoGeometry)?.rect;
        let positions = (0..layout_info.child_count)
            .map(|i| {
                self.hit_of(&Self::child_path(&layout, i)).map(|h| {
                    (snap(h.rect.x - origin.x, self.grid), snap(h.rect.y - origin.y, self.grid))
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(SessionError::NoGeometry)?;
        self.apply(Op::Unwrap { layout, positions })
    }

    /// Guides of the selected layout, or of the layout inside the selected container.
    pub fn guides(&self) -> Option<Guides> {
        let (path, info, kind) = self.guide_layout()?;
        let layout = self.hit_of(&path)?;
        let children: Vec<Hit> = (0..info.child_count)
            .map(|i| self.hit_of(&Self::child_path(&path, i)))
            .collect::<Option<_>>()?;
        Some(guides::compute(&layout, kind, &children, &info.bindings))
    }

    /// Edits a property of the guided layout, selecting the layout first when needed.
    pub fn set_guide_property(&mut self, name: &str, value: &str) -> Result<(), SessionError> {
        self.refuse_if_mismatch()?;
        let (path, ..) = self.guide_layout().ok_or(SessionError::NotALayout)?;
        let previous = self.selection.replace(path);
        let r = self.set_property(name, value);
        if r.is_err() {
            self.selection = previous;
        }
        r
    }

    pub fn tree_action(&mut self, path: &ElementPath, action: TreeAction) -> Result<(), SessionError> {
        let root_err = || {
            SessionError::Edit(EditError::InvalidTarget(
                "ルート要素は移動も削除もできません".into(),
            ))
        };
        let Some((&index, parent_indices)) = path.indices.split_last() else {
            return match action {
                TreeAction::NewContainer => self.insert_part(path.clone(), "Rectangle", None),
                _ => Err(root_err()),
            };
        };
        let parent = ElementPath { component: path.component.clone(), indices: parent_indices.to_vec() };
        let nothing = |msg: &str| SessionError::Edit(EditError::InvalidTarget(msg.into()));
        match action {
            TreeAction::Up => {
                if index == 0 {
                    return Err(nothing("これ以上、上へ移動できません"));
                }
                self.apply(Op::MoveElement {
                    path: path.clone(),
                    new_parent: parent,
                    index: Some(index - 1),
                })
            }
            TreeAction::Down => {
                let count = self.doc.element(&parent).map_or(0, |p| p.child_count);
                if index + 1 >= count {
                    return Err(nothing("これ以上、下へ移動できません"));
                }
                self.apply(Op::MoveElement {
                    path: path.clone(),
                    new_parent: parent,
                    index: Some(index + 1),
                })
            }
            TreeAction::Out => {
                let Some((&parent_index, grand)) = parent.indices.split_last() else {
                    return Err(nothing("ルート要素の外へは移動できません"));
                };
                let grandparent = ElementPath { component: path.component.clone(), indices: grand.to_vec() };
                self.apply(Op::MoveElement {
                    path: path.clone(),
                    new_parent: grandparent,
                    index: Some(parent_index + 1),
                })
            }
            TreeAction::In => {
                if index == 0 {
                    return Err(nothing("前の要素がないので、中へ入れられません"));
                }
                let mut sibling = parent;
                sibling.indices.push(index - 1);
                let info = self
                    .doc
                    .element(&sibling)
                    .ok_or_else(|| EditError::ElementNotFound(sibling.clone()))?;
                if !Self::is_container(&info) {
                    return Err(SessionError::NotAContainer);
                }
                self.apply(Op::MoveElement { path: path.clone(), new_parent: sibling, index: None })
            }
            TreeAction::Delete => {
                self.apply(Op::RemoveElement { path: path.clone() })?;
                self.selection = Some(parent);
                self.revalidate();
                Ok(())
            }
            TreeAction::NewContainer => self.insert_part(parent, "Rectangle", None),
        }
    }
}

/// A copy of `text` with every comment turned into spaces (newlines kept), byte for byte the
/// same length, so offsets found in it are valid in `text`. String literals are left alone.
fn mask_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let blank = |out: &mut String, c: char| {
        if c == '\n' {
            out.push('\n');
        } else {
            out.extend(std::iter::repeat_n(' ', c.len_utf8()));
        }
    };
    while let Some((_, c)) = chars.next() {
        match (c, chars.peek().map(|&(_, n)| n)) {
            ('/', Some('/')) => {
                blank(&mut out, c);
                while let Some(&(_, n)) = chars.peek() {
                    if n == '\n' {
                        break;
                    }
                    blank(&mut out, n);
                    chars.next();
                }
            }
            ('/', Some('*')) => {
                blank(&mut out, c);
                blank(&mut out, '*');
                chars.next();
                let mut prev = '\0';
                for (_, n) in chars.by_ref() {
                    blank(&mut out, n);
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            ('"', _) => {
                out.push(c);
                let mut escaped = false;
                for (_, n) in chars.by_ref() {
                    out.push(n);
                    if n == '"' && !escaped {
                        break;
                    }
                    escaped = n == '\\' && !escaped;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Adds `name` to the `import { ... } from "std-widgets.slint";` line, creating the line at the
/// top of the file when there is none. Everything else in the file stays byte-identical:
/// the name is inserted right after the last entry, matching a multi-line layout and a
/// trailing comma when the import uses them. An entry already imported (also under an alias,
/// `Button as B`) is left alone. Comments are ignored when looking for the import.
fn with_std_import(text: &str, name: &str) -> String {
    const MODULE: &str = "\"std-widgets.slint\"";
    let masked = mask_comments(text);
    let import = masked.match_indices(MODULE).find_map(|(from, _)| {
        let close = masked[..from].rfind('}')?;
        let open = masked[..close].rfind('{')?;
        masked[..open].trim_end().ends_with("import").then_some((open, close))
    });
    if let Some((open, close)) = import {
        let names = &masked[open + 1..close];
        if names.split(',').any(|n| n.split_whitespace().next() == Some(name)) {
            return text.to_string();
        }
        let (at, insert) = match names.rfind(|c: char| !c.is_whitespace()) {
            None if names.is_empty() => (open + 1, format!(" {name} ")),
            None => (close, format!("{name} ")),
            Some(last) => {
                let trailing_comma = names[last..].starts_with(',');
                let last_len = names[last..].chars().next().map_or(1, char::len_utf8);
                let at = open + 1 + last + last_len;
                let insert = if names.contains('\n') {
                    let line_start = names[..last].rfind('\n').map_or(0, |n| n + 1);
                    let line = &names[line_start..];
                    let indent = &line[..line.len() - line.trim_start().len()];
                    if trailing_comma {
                        format!("\n{indent}{name},")
                    } else {
                        format!(",\n{indent}{name}")
                    }
                } else if trailing_comma {
                    format!(" {name},")
                } else {
                    format!(", {name}")
                };
                (at, insert)
            }
        };
        return format!("{}{insert}{}", &text[..at], &text[at..]);
    }
    format!("import {{ {name} }} from {MODULE};\n{text}")
}
