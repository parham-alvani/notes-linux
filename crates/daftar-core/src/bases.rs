//! A `.base` file: an Obsidian Bases query over the vault's files. Filters
//! pick the files, formulas add columns, and each view sorts, groups and
//! shows them.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, LazyLock};

use chrono::{DateTime, Datelike, Local, Months, NaiveDate, TimeDelta, TimeZone, Timelike, Utc};
use regex::Regex;

use crate::base_expression::{BaseExpr, BaseParseError, BaseParser, BaseSpan, BaseValue, whole};
use crate::note_parser::Wikilink;
use crate::text::standard_compare;
use crate::vault::{Vault, VaultIndex};
use crate::yaml::{Yaml, YamlNode};

#[derive(Debug, Clone)]
pub struct BaseView {
    pub kind: String,
    pub name: String,
    pub filter: Option<BaseFilter>,
    pub order: Vec<String>,
    /// Each property, and whether it sorts ascending.
    pub sort: Vec<(String, bool)>,
    pub group_by: Option<(String, bool)>,
    pub limit: Option<i64>,
    /// How a `cards` or `list` view draws its rows.
    pub layout: BaseLayout,
}

impl Default for BaseView {
    fn default() -> BaseView {
        BaseView {
            kind: "table".into(),
            name: "Table".into(),
            filter: None,
            order: vec![],
            sort: vec![],
            group_by: None,
            limit: None,
            layout: BaseLayout::default(),
        }
    }
}

impl BaseView {
    /// The way the page draws the view: `table`, `cards` or `list`. A view
    /// type the app does not know, a plugin's, is a table.
    pub fn draws_as(&self) -> &'static str {
        match self.kind.as_str() {
            "cards" => "cards",
            "list" => "list",
            _ => "table",
        }
    }
}

/// The options of a `cards` or a `list` view, as Obsidian writes them in the
/// view: `image`, `imageFit`, `imageAspectRatio` and `cardSize` for cards;
/// `markers`, `indentProperties` and `separator` for a list. A value out of
/// range or of the wrong kind keeps the default.
#[derive(Debug, Clone, PartialEq)]
pub struct BaseLayout {
    /// The property whose value is a card's cover: an image in the vault, a
    /// web address or a `#hex` colour.
    pub image: Option<String>,
    /// `cover` crops the image to fill the cover; `contain` shows all of it.
    pub image_fit: String,
    /// The cover's height over its width.
    pub image_aspect_ratio: f64,
    /// The width of a card, in points.
    pub card_size: f64,
    /// `bullet`, `number` or `none`.
    pub markers: String,
    /// Each property on its own line under the first, not on one line.
    pub indent_properties: bool,
    pub separator: String,
}

impl Default for BaseLayout {
    fn default() -> BaseLayout {
        BaseLayout {
            image: None,
            image_fit: "cover".into(),
            image_aspect_ratio: 1.0,
            card_size: 200.0,
            markers: "bullet".into(),
            indent_properties: false,
            separator: ",".into(),
        }
    }
}

impl BaseLayout {
    fn new(node: &YamlNode) -> BaseLayout {
        let mut layout = BaseLayout::default();
        let text = |key: &str| node.get(key).and_then(YamlNode::string);
        let number = |key: &str| text(key).and_then(|t| t.trim().parse::<f64>().ok()).filter(|n| n.is_finite() && *n > 0.0);
        layout.image = text("image").filter(|i| !i.is_empty()).map(String::from);
        if let Some(fit) = text("imageFit").map(str::to_lowercase).filter(|f| f == "cover" || f == "contain") {
            layout.image_fit = fit;
        }
        if let Some(ratio) = number("imageAspectRatio") {
            layout.image_aspect_ratio = ratio.clamp(0.1, 10.0);
        }
        if let Some(size) = number("cardSize") {
            layout.card_size = size.clamp(50.0, 1000.0);
        }
        match text("markers").map(str::to_lowercase).as_deref() {
            Some("number" | "numbers") => layout.markers = "number".into(),
            Some("none") => layout.markers = "none".into(),
            _ => {}
        }
        layout.indent_properties = text("indentProperties").is_some_and(|t| t.eq_ignore_ascii_case("true"));
        if let Some(mark) = text("separator").filter(|m| !m.is_empty()) {
            layout.separator = mark.to_string();
        }
        layout
    }
}

/// A card's cover: an image in the vault, one on the web, or a colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseImage {
    File(String),
    Url(String),
    Color(String),
}

#[derive(Debug, Clone, Default)]
pub struct Base {
    pub filter: Option<BaseFilter>,
    pub formulas: Vec<(String, String)>,
    pub display_names: HashMap<String, String>,
    pub views: Vec<BaseView>,
}

impl Base {
    pub fn parse(source: &str) -> Base {
        let root = Yaml::parse(source);
        let mut base = Base { filter: root.get("filters").and_then(BaseFilter::new), ..Base::default() };
        if let Some(formulas) = root.get("formulas") {
            base.formulas = formulas.entries().iter().filter_map(|(k, v)| v.string().map(|s| (k.clone(), s.to_string()))).collect();
        }
        if let Some(properties) = root.get("properties") {
            for (key, value) in properties.entries() {
                if let Some(name) = value.get("displayName").and_then(YamlNode::string) {
                    base.display_names.insert(key.clone(), name.to_string());
                }
            }
        }
        let descending = |node: Option<&YamlNode>| node.and_then(YamlNode::string).is_some_and(|d| d.eq_ignore_ascii_case("DESC"));
        for node in root.get("views").map(YamlNode::items).unwrap_or_default() {
            let mut view = BaseView {
                kind: node.get("type").and_then(YamlNode::string).unwrap_or("table").to_string(),
                name: node.get("name").and_then(YamlNode::string).map_or(format!("View {}", base.views.len() + 1), String::from),
                filter: node.get("filters").and_then(BaseFilter::new),
                ..BaseView::default()
            };
            view.order =
                node.get("order").map(YamlNode::items).unwrap_or_default().iter().filter_map(|i| i.string().map(String::from)).collect();
            view.sort = node
                .get("sort")
                .map(YamlNode::items)
                .unwrap_or_default()
                .iter()
                .filter_map(|item| {
                    item.get("property").and_then(YamlNode::string).map(|p| (p.to_string(), !descending(item.get("direction"))))
                })
                .collect();
            if let Some(group) = node.get("groupBy")
                && let Some(property) = group.get("property").and_then(YamlNode::string).or(group.string())
            {
                view.group_by = Some((property.to_string(), !descending(group.get("direction"))));
            }
            view.limit = node.get("limit").and_then(YamlNode::string).and_then(|l| l.parse().ok());
            view.layout = BaseLayout::new(node);
            base.views.push(view);
        }
        if base.views.is_empty() {
            base.views.push(BaseView::default());
        }
        base
    }
}

/// `filters:` -- one statement, or `and`, `or` and `not` over a list of them.
#[derive(Debug, Clone)]
pub enum BaseFilter {
    Statement(String),
    All(Vec<BaseFilter>),
    Any(Vec<BaseFilter>),
    None(Vec<BaseFilter>),
}

impl BaseFilter {
    fn new(node: &YamlNode) -> Option<BaseFilter> {
        match node {
            YamlNode::Scalar(text) => Some(BaseFilter::Statement(text.clone())),
            YamlNode::List(items) => Some(BaseFilter::All(items.iter().filter_map(BaseFilter::new).collect())),
            YamlNode::Map(entries) => {
                let mut parts: Vec<BaseFilter> = entries
                    .iter()
                    .filter_map(|(key, value)| {
                        let items = value.items().into_iter().filter_map(BaseFilter::new).collect();
                        match key.as_str() {
                            "and" => Some(BaseFilter::All(items)),
                            "or" => Some(BaseFilter::Any(items)),
                            "not" => Some(BaseFilter::None(items)),
                            _ => None,
                        }
                    })
                    .collect();
                match parts.len() {
                    0 => None,
                    1 => parts.pop(),
                    _ => Some(BaseFilter::All(parts)),
                }
            }
            YamlNode::Null => None,
        }
    }
}

/// What a view shows: the columns, then the rows in their groups.
#[derive(Debug, Clone, PartialEq)]
pub struct BaseTable {
    pub name: String,
    /// The view's `type` as written.
    pub kind: String,
    /// `table`, `cards` or `list`: how the page draws it.
    pub draws_as: String,
    pub layout: BaseLayout,
    pub columns: Vec<String>,
    pub groups: Vec<BaseGroup>,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BaseGroup {
    /// `None` when the view has no `groupBy`.
    pub name: Option<String>,
    pub rows: Vec<BaseRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BaseRow {
    pub path: String,
    pub cells: Vec<BaseCell>,
    /// The card's cover, in a `cards` view with an `image`.
    pub image: Option<BaseImage>,
}

/// One cell as the page draws it: text runs, some of them links.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BaseCell {
    pub parts: Vec<CellPart>,
    /// A checkbox property.
    pub check: Option<bool>,
    pub numeric: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellPart {
    pub text: String,
    pub path: Option<String>,
}

impl CellPart {
    fn text(text: impl Into<String>) -> CellPart {
        CellPart { text: text.into(), path: None }
    }
}

impl BaseCell {
    pub fn text(&self) -> String {
        self.parts.iter().map(|p| p.text.as_str()).collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Stat {
    pub created: DateTime<Local>,
    pub modified: DateTime<Local>,
    pub size: u64,
}

#[derive(Default)]
struct RowState {
    formulas: HashMap<String, BaseValue>,
    working: HashSet<String>,
}

type Locals = HashMap<String, BaseValue>;
/// A file's dates and size, by vault path.
type StatOf<'a> = Box<dyn Fn(&str) -> Option<Stat> + 'a>;

/// Runs a base over a vault index. One engine is for one run: it keeps what
/// it has worked out for each file, so the views share it.
pub struct BaseEngine<'a> {
    index: &'a VaultIndex,
    now: DateTime<Local>,
    /// `.obsidian/types.json`: a property's type, which says whether `2026`
    /// is a number or text and whether one value is still a list.
    types: HashMap<String, String>,
    stat: StatOf<'a>,
    stats: HashMap<String, Option<Stat>>,
    formulas: HashMap<String, Arc<BaseExpr>>,
    rows: HashMap<String, RowState>,
    parsed: HashMap<String, Result<Arc<BaseExpr>, BaseParseError>>,
    /// The file `this` is: the base, or the note that embeds it.
    context: String,
    /// What could not be read or worked out, once each, for the page to list.
    pub problems: Vec<String>,
}

static DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.\d+)?)?)?(Z)?$").unwrap());
static FORMAT_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[[^\]]*\]|YYYY|YY|Q|MMMM|MMM|MM|M|Do|DD|D|dddd|ddd|dd|d|HH|H|hh|h|mm|m|ss|s|SSS|A|a|ZZ|Z|X|x").unwrap());

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const LIST_TYPES: [&str; 3] = ["multitext", "tags", "aliases"];
const IMAGE_TYPES: [&str; 11] = ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif", "heic", "tif", "tiff"];
static HEX_COLOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$").unwrap());

impl<'a> BaseEngine<'a> {
    pub fn new(
        index: &'a VaultIndex,
        now: DateTime<Local>,
        types: HashMap<String, String>,
        stat: impl Fn(&str) -> Option<Stat> + 'a,
    ) -> Self {
        BaseEngine {
            index,
            now,
            types,
            stat: Box::new(stat),
            stats: HashMap::new(),
            formulas: HashMap::new(),
            rows: HashMap::new(),
            parsed: HashMap::new(),
            context: String::new(),
            problems: Vec::new(),
        }
    }

    /// An engine for the vault at [root] now: its property types and its
    /// files' dates and sizes come from the disk.
    pub fn on_disk(index: &'a VaultIndex, root: &'a Path) -> Self {
        BaseEngine::new(index, Local::now(), BaseEngine::types(root), move |path| BaseEngine::stat_file(root, path))
    }

    /// The property types in a vault's `.obsidian/types.json`.
    pub fn types(root: &Path) -> HashMap<String, String> {
        let Ok(text) = std::fs::read_to_string(root.join(".obsidian/types.json")) else { return HashMap::new() };
        let Ok(serde_json::Value::Object(object)) = serde_json::from_str(&text) else { return HashMap::new() };
        match object.get("types") {
            Some(serde_json::Value::Object(types)) => {
                types.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect()
            }
            _ => HashMap::new(),
        }
    }

    /// A file's dates and size from the disk; the creation date falls back to
    /// the change date where the file system keeps none.
    pub fn stat_file(root: &Path, path: &str) -> Option<Stat> {
        let meta = std::fs::metadata(root.join(path)).ok()?;
        let modified: DateTime<Local> = meta.modified().ok()?.into();
        let created = meta.created().map(DateTime::<Local>::from).unwrap_or(modified);
        Some(Stat { created, modified, size: meta.len() })
    }

    /// Every view of [base]; [context] is the file `this` names.
    pub fn run(&mut self, base: &Base, context: &str) -> Vec<BaseTable> {
        self.context = context.to_string();
        for (name, source) in &base.formulas {
            if let Some(expression) = self.expression(source, &format!("Formula {name}")) {
                self.formulas.insert(name.clone(), expression);
            }
        }
        let files: Vec<String> = self.index.files.iter().cloned().collect();
        let candidates: Vec<String> = files.into_iter().filter(|p| self.passes(base.filter.as_ref(), p).unwrap_or(true)).collect();
        base.views.iter().map(|view| self.table(view, base, &candidates)).collect()
    }

    fn table(&mut self, view: &BaseView, base: &Base, candidates: &[String]) -> BaseTable {
        let mut paths: Vec<String> = candidates.iter().filter(|p| self.passes(view.filter.as_ref(), p).unwrap_or(true)).cloned().collect();
        if !view.sort.is_empty() {
            let keys: Vec<(BaseExpr, bool)> = view.sort.iter().map(|(p, asc)| (column(p), *asc)).collect();
            let values: HashMap<String, Vec<BaseValue>> =
                paths.iter().map(|p| (p.clone(), keys.iter().map(|(k, _)| self.value_of(k, p)).collect())).collect();
            safe_sort(&mut paths, |a, b| {
                for (offset, (_, ascending)) in keys.iter().enumerate() {
                    let order = order(&values[a][offset], &values[b][offset], *ascending);
                    if order != 0 {
                        return order < 0;
                    }
                }
                a < b
            });
        }
        if let Some(limit) = view.limit
            && limit >= 0
        {
            paths.truncate(limit as usize);
        }
        let order_ids = if view.order.is_empty() { vec!["file.name".to_string()] } else { view.order.clone() };
        let columns: Vec<BaseExpr> = order_ids.iter().map(|id| column(id)).collect();
        let cover = if view.draws_as() == "cards" { view.layout.image.as_deref().map(column) } else { None };
        let rows: Vec<BaseRow> = paths
            .iter()
            .map(|path| BaseRow {
                path: path.clone(),
                cells: order_ids.iter().zip(&columns).map(|(id, expression)| self.cell_at(id, expression, path)).collect(),
                image: cover.as_ref().and_then(|c| {
                    let value = self.value_of(c, path);
                    self.image(&value, path)
                }),
            })
            .collect();
        let count = rows.len();
        let groups = if let Some((property, ascending)) = &view.group_by {
            let key = column(property);
            let mut by_name: HashMap<String, (BaseValue, Vec<BaseRow>)> = HashMap::new();
            let mut names: Vec<String> = Vec::new();
            for row in rows {
                let value = self.value_of(&key, &row.path);
                let name = if is_empty(&value) { String::new() } else { self.display(&value) };
                by_name
                    .entry(name.clone())
                    .or_insert_with(|| {
                        names.push(name);
                        (value, Vec::new())
                    })
                    .1
                    .push(row);
            }
            safe_sort(&mut names, |a, b| {
                let o = order(&by_name[a].0, &by_name[b].0, *ascending);
                if o != 0 { o < 0 } else { a < b }
            });
            names
                .into_iter()
                .map(|name| {
                    let rows = by_name.remove(&name).map(|g| g.1).unwrap_or_default();
                    BaseGroup { name: Some(if name.is_empty() { "None".into() } else { name }), rows }
                })
                .collect()
        } else {
            vec![BaseGroup { name: None, rows }]
        };
        BaseTable {
            name: view.name.clone(),
            kind: view.kind.clone(),
            draws_as: view.draws_as().into(),
            layout: view.layout.clone(),
            columns: order_ids.iter().map(|id| base.display_names.get(id).cloned().unwrap_or_else(|| label(id))).collect(),
            groups,
            count,
        }
    }

    /// A cover from a property's value: a link to a picture, a path or a
    /// name the vault resolves, a web address, or a `#hex` colour. The first
    /// of a list. A link to a note is no cover.
    fn image(&self, value: &BaseValue, from: &str) -> Option<BaseImage> {
        let picture = |file: &str| {
            let extension = file.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
            IMAGE_TYPES.contains(&extension.as_str()).then(|| BaseImage::File(file.to_string()))
        };
        match value {
            BaseValue::File(file) => picture(file),
            BaseValue::Link { path: Some(file), .. } => picture(file),
            BaseValue::Link { target, path: None, .. } => self.image(&BaseValue::String(target.clone()), from),
            BaseValue::List(items) => items.first().and_then(|i| self.image(i, from)),
            BaseValue::String(raw) => {
                let mut text = raw.trim().to_string();
                if HEX_COLOR.is_match(&text) {
                    return Some(BaseImage::Color(text));
                }
                let lowered = text.to_lowercase();
                if lowered.starts_with("https://") || lowered.starts_with("http://") {
                    return Some(BaseImage::Url(text));
                }
                if let Some(rest) = text.strip_prefix('!') {
                    text = rest.to_string();
                }
                if let Some(inner) = text.strip_prefix("[[").and_then(|t| t.strip_suffix("]]")) {
                    text = Wikilink::new(inner).target;
                }
                self.index.resolve(&text, from).and_then(|p| picture(&p))
            }
            _ => None,
        }
    }

    // Filters and columns

    /// `None` when nothing in [filter] could be applied.
    fn passes(&mut self, filter: Option<&BaseFilter>, path: &str) -> Option<bool> {
        let filter = filter?;
        let mut results = |items: &[BaseFilter]| -> Vec<bool> { items.iter().filter_map(|f| self.passes(Some(f), path)).collect() };
        match filter {
            BaseFilter::Statement(source) => {
                let expression = self.expression(source, "Filter")?;
                Some(truthy(&self.evaluate(&expression, path, &Locals::new())))
            }
            BaseFilter::All(items) => {
                let r = results(items);
                (!r.is_empty()).then(|| !r.contains(&false))
            }
            BaseFilter::Any(items) => {
                let r = results(items);
                (!r.is_empty()).then(|| r.contains(&true))
            }
            BaseFilter::None(items) => {
                let r = results(items);
                (!r.is_empty()).then(|| !r.contains(&true))
            }
        }
    }

    fn value_of(&mut self, expression: &BaseExpr, path: &str) -> BaseValue {
        self.evaluate(expression, path, &Locals::new())
    }

    fn cell_at(&mut self, id: &str, expression: &BaseExpr, path: &str) -> BaseCell {
        // The name column is the file, as a link to it.
        if id == "file.name" || id == "file.basename" || id == "file" {
            return BaseCell { parts: vec![CellPart { text: title(path), path: Some(path.into()) }], ..BaseCell::default() };
        }
        let value = self.value_of(expression, path);
        self.cell(&value)
    }

    pub fn cell(&self, value: &BaseValue) -> BaseCell {
        match value {
            BaseValue::Null => BaseCell::default(),
            BaseValue::Bool(flag) => BaseCell { check: Some(*flag), ..BaseCell::default() },
            BaseValue::Number(_) => BaseCell { parts: vec![CellPart::text(self.display(value))], numeric: true, ..BaseCell::default() },
            BaseValue::List(items) => {
                let mut cell = BaseCell::default();
                for (offset, item) in items.iter().enumerate() {
                    if offset > 0 {
                        cell.parts.push(CellPart::text(", "));
                    }
                    cell.parts.extend(self.cell(item).parts);
                }
                cell
            }
            BaseValue::Link { target, path, display } => {
                let text = display.clone().or_else(|| path.as_deref().map(title)).unwrap_or_else(|| target.clone());
                BaseCell { parts: vec![CellPart { text, path: path.clone() }], ..BaseCell::default() }
            }
            BaseValue::File(path) => {
                BaseCell { parts: vec![CellPart { text: title(path), path: Some(path.clone()) }], ..BaseCell::default() }
            }
            _ => BaseCell { parts: vec![CellPart::text(self.display(value))], ..BaseCell::default() },
        }
    }

    fn expression(&mut self, source: &str, what: &str) -> Option<Arc<BaseExpr>> {
        let parsed = self.parsed.entry(source.to_string()).or_insert_with(|| BaseParser::parse(source).map(Arc::new)).clone();
        match parsed {
            Ok(expression) => Some(expression),
            Err(error) => {
                self.note(format!("{what} not understood, so not applied: {source} ({})", error.message));
                None
            }
        }
    }

    fn note(&mut self, problem: String) {
        if !self.problems.contains(&problem) {
            self.problems.push(problem);
        }
    }

    fn file_stat(&mut self, path: &str) -> Option<Stat> {
        if let Some(known) = self.stats.get(path) {
            return *known;
        }
        let found = (self.stat)(path);
        self.stats.insert(path.to_string(), found);
        found
    }

    // Evaluation

    pub fn evaluate(&mut self, expression: &BaseExpr, path: &str, locals: &Locals) -> BaseValue {
        match expression {
            BaseExpr::Value(value) => value.clone(),
            BaseExpr::List(items) => BaseValue::List(items.iter().map(|i| self.evaluate(i, path, locals)).collect()),
            BaseExpr::Name(name) => {
                if let Some(local) = locals.get(name) {
                    return local.clone();
                }
                match name.as_str() {
                    "file" => BaseValue::File(path.into()),
                    "this" => BaseValue::File(self.context.clone()),
                    "note" => BaseValue::Object(self.properties(path)),
                    "formula" => {
                        let mut names: Vec<String> = self.formulas.keys().cloned().collect();
                        names.sort();
                        BaseValue::Object(names.into_iter().map(|n| (n.clone(), self.formula(&n, path))).collect())
                    }
                    _ => self.property(name, path),
                }
            }
            BaseExpr::Member(receiver, name) => {
                if let BaseExpr::Name(space) = receiver.as_ref()
                    && !locals.contains_key(space)
                {
                    if space == "formula" {
                        return self.formula(name, path);
                    }
                    if space == "note" {
                        return self.property(name, path);
                    }
                }
                let value = self.evaluate(receiver, path, locals);
                self.member(&value, name)
            }
            BaseExpr::Index(receiver, key) => {
                let key = self.evaluate(key, path, locals);
                if let BaseExpr::Name(space) = receiver.as_ref()
                    && space == "note"
                    && !locals.contains_key("note")
                    && let BaseValue::String(name) = &key
                {
                    return self.property(name, path);
                }
                let value = self.evaluate(receiver, path, locals);
                self.subscript(&value, &key)
            }
            BaseExpr::Call(name, arguments) => self.call(name, arguments, path, locals),
            BaseExpr::Method(receiver, name, arguments) => {
                let value = self.evaluate(receiver, path, locals);
                self.method(value, name, arguments, path, locals)
            }
            BaseExpr::Unary(symbol, operand) => {
                let value = self.evaluate(operand, path, locals);
                if symbol == "!" {
                    return BaseValue::Bool(!truthy(&value));
                }
                match value {
                    BaseValue::Number(n) => BaseValue::Number(-n),
                    BaseValue::Duration(span) => BaseValue::Duration(span.negated()),
                    _ => BaseValue::Null,
                }
            }
            BaseExpr::Binary(symbol, left, right) => {
                if symbol == "&&" {
                    return BaseValue::Bool(truthy(&self.evaluate(left, path, locals)) && truthy(&self.evaluate(right, path, locals)));
                }
                if symbol == "||" {
                    return BaseValue::Bool(truthy(&self.evaluate(left, path, locals)) || truthy(&self.evaluate(right, path, locals)));
                }
                let (l, r) = (self.evaluate(left, path, locals), self.evaluate(right, path, locals));
                self.binary(symbol, &l, &r)
            }
        }
    }

    fn formula(&mut self, name: &str, path: &str) -> BaseValue {
        let row = self.rows.entry(path.to_string()).or_default();
        if let Some(known) = row.formulas.get(name) {
            return known.clone();
        }
        let Some(expression) = self.formulas.get(name).cloned() else { return BaseValue::Null };
        if !row.working.insert(name.to_string()) {
            self.note(format!("Formula {name} uses itself"));
            return BaseValue::Null;
        }
        let value = self.evaluate(&expression, path, &Locals::new());
        let row = self.rows.entry(path.to_string()).or_default();
        row.working.remove(name);
        row.formulas.insert(name.to_string(), value.clone());
        value
    }

    // Properties

    fn properties(&self, path: &str) -> Vec<(String, BaseValue)> {
        let Some(note) = self.index.notes.get(path) else { return vec![] };
        note.parsed.properties.iter().map(|(k, v)| (k.clone(), self.convert(k, v, path))).collect()
    }

    pub fn property(&self, key: &str, path: &str) -> BaseValue {
        let Some(note) = self.index.notes.get(path) else { return BaseValue::Null };
        let properties = &note.parsed.properties;
        let lowered = key.to_lowercase();
        let found = properties.iter().find(|(k, _)| k == key).or_else(|| properties.iter().find(|(k, _)| k.to_lowercase() == lowered));
        match found {
            Some((k, values)) => self.convert(k, values, path),
            None => BaseValue::Null,
        }
    }

    /// Front matter is read as text; the type in `types.json`, or the shape
    /// of the text, makes it a number, a date, a checkbox, a link or a list.
    fn convert(&self, key: &str, values: &[String], path: &str) -> BaseValue {
        let kind = self.types.get(key).map(String::as_str);
        let list = kind.is_some_and(|k| LIST_TYPES.contains(&k));
        if values.is_empty() {
            return if list { BaseValue::List(vec![]) } else { BaseValue::Null };
        }
        if values.len() > 1 || list {
            return BaseValue::List(values.iter().map(|v| self.scalar(v, Some("text"), path)).collect());
        }
        self.scalar(&values[0], kind, path)
    }

    fn scalar(&self, text: &str, kind: Option<&str>, path: &str) -> BaseValue {
        if text.len() >= 4 && text.starts_with("[[") && text.ends_with("]]") {
            let link = Wikilink::new(&text[2..text.len() - 2]);
            return BaseValue::Link { path: self.index.resolve(&link.target, path), target: link.target, display: link.alias };
        }
        match kind {
            Some("number") => text.parse().map(BaseValue::Number).unwrap_or_else(|_| BaseValue::String(text.into())),
            Some("checkbox") => BaseValue::Bool(text.to_lowercase() == "true"),
            Some("date" | "datetime") => parse_date(text).map_or(BaseValue::String(text.into()), |(d, t)| BaseValue::Date(d, t)),
            Some("text") => BaseValue::String(text.into()),
            _ => {
                if text == "true" || text == "false" {
                    return BaseValue::Bool(text == "true");
                }
                if let Ok(number) = text.parse::<f64>()
                    && (text.starts_with(|c: char| c.is_ascii_digit()) || text.starts_with('-'))
                {
                    return BaseValue::Number(number);
                }
                if let Some((date, time)) = parse_date(text) {
                    return BaseValue::Date(date, time);
                }
                BaseValue::String(text.into())
            }
        }
    }

    // Members

    fn member(&mut self, value: &BaseValue, name: &str) -> BaseValue {
        let number = |n: f64| BaseValue::Number(n);
        match value {
            BaseValue::File(path) => self.file_member(path, name),
            BaseValue::Link { path: Some(path), .. } if name != "length" => self.file_member(path, name),
            BaseValue::String(text) if name == "length" => number(text.chars().count() as f64),
            BaseValue::List(items) if name == "length" => number(items.len() as f64),
            BaseValue::Date(date, _) => match name {
                "year" => number(date.year() as f64),
                "month" => number(date.month() as f64),
                "day" => number(date.day() as f64),
                "hour" => number(date.hour() as f64),
                "minute" => number(date.minute() as f64),
                "second" => number(date.second() as f64),
                "millisecond" => number((date.nanosecond() / 1_000_000) as f64),
                _ => BaseValue::Null,
            },
            BaseValue::Duration(span) => {
                let ms = span.approximate_milliseconds();
                match name {
                    "days" => number(ms / 86_400_000.0),
                    "hours" => number(ms / 3_600_000.0),
                    "minutes" => number(ms / 60_000.0),
                    "seconds" => number(ms / 1000.0),
                    "milliseconds" => number(ms),
                    _ => BaseValue::Null,
                }
            }
            BaseValue::Object(entries) => entries.iter().find(|(k, _)| k == name).map_or(BaseValue::Null, |(_, v)| v.clone()),
            _ => BaseValue::Null,
        }
    }

    fn file_member(&mut self, path: &str, name: &str) -> BaseValue {
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let dot = file_name.rfind('.');
        let string = |s: &str| BaseValue::String(s.into());
        match name {
            "name" => string(file_name),
            "basename" => string(dot.map_or(file_name, |d| &file_name[..d])),
            "ext" => string(dot.map_or("", |d| &file_name[d + 1..])),
            "path" => string(path),
            "folder" => string(path.rfind('/').map_or("/", |slash| &path[..slash])),
            "file" => BaseValue::File(path.into()),
            "mtime" => {
                if let Some(note) = self.index.notes.get(path) {
                    return BaseValue::Date(note.modified.into(), true);
                }
                self.file_stat(path).map_or(BaseValue::Null, |s| BaseValue::Date(s.modified, true))
            }
            "ctime" => self.file_stat(path).map_or(BaseValue::Null, |s| BaseValue::Date(s.created, true)),
            "size" => self.file_stat(path).map_or(BaseValue::Null, |s| BaseValue::Number(s.size as f64)),
            "properties" => BaseValue::Object(self.properties(path)),
            "tags" => BaseValue::List(
                self.index
                    .notes
                    .get(path)
                    .map(|n| n.parsed.tags.iter().map(|t| BaseValue::String(t.clone())).collect())
                    .unwrap_or_default(),
            ),
            "links" | "embeds" => {
                let mut seen = HashSet::new();
                let links = self.index.notes.get(path).map(|n| n.parsed.links.as_slice()).unwrap_or(&[]);
                BaseValue::List(
                    links
                        .iter()
                        .filter(|l| (name == "embeds") == l.embed && seen.insert(l.target.clone()))
                        .map(|l| BaseValue::Link { target: l.target.clone(), path: self.index.resolve(&l.target, path), display: None })
                        .collect(),
                )
            }
            "backlinks" => {
                let mut seen = HashSet::new();
                BaseValue::List(
                    self.index
                        .backlinks(path)
                        .iter()
                        .filter(|b| seen.insert(b.source.clone()))
                        .map(|b| BaseValue::File(b.source.clone()))
                        .collect(),
                )
            }
            _ => BaseValue::Null,
        }
    }

    fn subscript(&mut self, value: &BaseValue, key: &BaseValue) -> BaseValue {
        match (value, key) {
            (BaseValue::List(items), BaseValue::Number(n)) => {
                let Some(w) = whole(*n) else { return BaseValue::Null };
                let at = if w < 0 { items.len() as i64 + w } else { w };
                usize::try_from(at).ok().and_then(|at| items.get(at)).cloned().unwrap_or(BaseValue::Null)
            }
            (BaseValue::String(text), BaseValue::Number(n)) => {
                let Some(at) = whole(*n).and_then(|w| usize::try_from(w).ok()) else { return BaseValue::Null };
                text.chars().nth(at).map_or(BaseValue::Null, |c| BaseValue::String(c.to_string()))
            }
            (_, BaseValue::String(name)) => self.member(value, name),
            _ => BaseValue::Null,
        }
    }

    // Functions

    fn call(&mut self, name: &str, arguments: &[BaseExpr], path: &str, locals: &Locals) -> BaseValue {
        if name == "if" {
            let Some(condition) = arguments.first() else { return BaseValue::Null };
            let pick = if truthy(&self.evaluate(condition, path, locals)) { 1 } else { 2 };
            return arguments.get(pick).map_or(BaseValue::Null, |a| self.evaluate(a, path, locals));
        }
        let values: Vec<BaseValue> = arguments.iter().map(|a| self.evaluate(a, path, locals)).collect();
        let first = values.first().cloned().unwrap_or(BaseValue::Null);
        match name {
            "now" => BaseValue::Date(self.now, true),
            "today" => BaseValue::Date(start_of_day(self.now), false),
            "date" => match &first {
                BaseValue::Date(..) => first,
                BaseValue::String(text) => parse_date(text).map_or(BaseValue::Null, |(d, t)| BaseValue::Date(d, t)),
                _ => BaseValue::Null,
            },
            "duration" => match &first {
                BaseValue::String(text) => BaseSpan::parse(text).map_or(BaseValue::Null, BaseValue::Duration),
                _ => first,
            },
            "number" => number(&first).map_or(BaseValue::Null, BaseValue::Number),
            "list" => match first {
                BaseValue::List(_) => first,
                _ if is_empty(&first) && values.is_empty() => BaseValue::List(vec![]),
                _ => BaseValue::List(vec![first]),
            },
            "link" => {
                let shown = values.get(1).map(|v| self.display(v));
                match first {
                    BaseValue::String(target) => BaseValue::Link { path: self.resolve(&target), target, display: shown },
                    BaseValue::File(path) => BaseValue::Link { target: path.clone(), path: Some(path), display: shown },
                    BaseValue::Link { target, path, .. } => BaseValue::Link { target, path, display: shown },
                    _ => BaseValue::Null,
                }
            }
            "file" => match first {
                BaseValue::String(target) => self.resolve(&target).map_or(BaseValue::Null, BaseValue::File),
                BaseValue::Link { path: Some(path), .. } => BaseValue::File(path),
                BaseValue::File(_) => first,
                _ => BaseValue::Null,
            },
            "max" | "min" => {
                let numbers: Vec<f64> = values.iter().filter_map(number).collect();
                if numbers.is_empty() {
                    return BaseValue::Null;
                }
                let fold = if name == "max" { f64::max } else { f64::min };
                BaseValue::Number(numbers.iter().copied().reduce(fold).unwrap_or(0.0))
            }
            "escapeHTML" | "html" | "icon" | "image" => BaseValue::String(self.display(&first)),
            "random" => BaseValue::Number(random()),
            _ => {
                self.note(format!("Unknown function {name}()"));
                BaseValue::Null
            }
        }
    }

    fn resolve(&self, target: &str) -> Option<String> {
        if self.index.files.contains(target) {
            return Some(target.to_string());
        }
        self.index.resolve(target, &self.context)
    }

    fn method(&mut self, receiver: BaseValue, name: &str, arguments: &[BaseExpr], path: &str, locals: &Locals) -> BaseValue {
        // These run their argument once per item, with `value` and `index` set.
        if let BaseValue::List(items) = &receiver
            && ["filter", "map", "reduce"].contains(&name)
            && let Some(body) = arguments.first()
        {
            let scope = |item: &BaseValue, offset: usize| {
                let mut scope = locals.clone();
                scope.insert("value".into(), item.clone());
                scope.insert("index".into(), BaseValue::Number(offset as f64));
                scope
            };
            return match name {
                "filter" => BaseValue::List(
                    items
                        .iter()
                        .enumerate()
                        .filter(|(o, item)| truthy(&self.evaluate(body, path, &scope(item, *o))))
                        .map(|(_, i)| i.clone())
                        .collect(),
                ),
                "map" => BaseValue::List(items.iter().enumerate().map(|(o, item)| self.evaluate(body, path, &scope(item, o))).collect()),
                _ => {
                    let mut accumulator = arguments.get(1).map_or(BaseValue::Null, |a| self.evaluate(a, path, locals));
                    for (offset, item) in items.iter().enumerate() {
                        let mut s = scope(item, offset);
                        s.insert("acc".into(), accumulator);
                        accumulator = self.evaluate(body, path, &s);
                    }
                    accumulator
                }
            };
        }
        let values: Vec<BaseValue> = arguments.iter().map(|a| self.evaluate(a, path, locals)).collect();
        let first = values.first().cloned().unwrap_or(BaseValue::Null);
        // Methods every value has.
        match name {
            "isEmpty" => return BaseValue::Bool(is_empty(&receiver)),
            "isTruthy" => return BaseValue::Bool(truthy(&receiver)),
            "toString" => return BaseValue::String(self.display(&receiver)),
            "isType" => return BaseValue::Bool(type_name(&receiver) == self.display(&first).to_lowercase()),
            _ => {}
        }
        let digits = |value: &BaseValue, low: i64, high: i64| number(value).and_then(whole).unwrap_or(0).clamp(low, high);
        match &receiver {
            BaseValue::Null => return BaseValue::Null,
            BaseValue::String(text) => return self.string_method(text, name, &values),
            BaseValue::Number(n) => match name {
                "abs" => return BaseValue::Number(n.abs()),
                "ceil" => return BaseValue::Number(n.ceil()),
                "floor" => return BaseValue::Number(n.floor()),
                "round" => {
                    let scale = 10f64.powi(digits(&first, -100, 100) as i32);
                    return BaseValue::Number((n * scale).round() / scale);
                }
                "toFixed" => return BaseValue::String(format!("{:.*}", digits(&first, 0, 100) as usize, n)),
                _ => {}
            },
            BaseValue::Date(date, time) => match name {
                "format" => return BaseValue::String(format_date(*date, &self.display(&first))),
                "relative" => return BaseValue::String(relative(*date, self.now)),
                "date" => return BaseValue::Date(start_of_day(*date), false),
                "time" => return BaseValue::String(if *time { format_date(*date, "HH:mm:ss") } else { "00:00:00".into() }),
                _ => {}
            },
            BaseValue::List(items) => match name {
                "contains" => return BaseValue::Bool(items.iter().any(|i| equal(i, &first))),
                "containsAll" => return BaseValue::Bool(values.iter().all(|v| items.iter().any(|i| equal(i, v)))),
                "containsAny" => return BaseValue::Bool(values.iter().any(|v| items.iter().any(|i| equal(i, v)))),
                "join" => {
                    let separator = if values.is_empty() { ",".to_string() } else { self.display(&first) };
                    return BaseValue::String(items.iter().map(|i| self.display(i)).collect::<Vec<_>>().join(&separator));
                }
                "reverse" => return BaseValue::List(items.iter().rev().cloned().collect()),
                "flat" => {
                    return BaseValue::List(
                        items
                            .iter()
                            .flat_map(|i| match i {
                                BaseValue::List(inner) => inner.clone(),
                                other => vec![other.clone()],
                            })
                            .collect(),
                    );
                }
                "sort" => {
                    let mut sorted = items.clone();
                    safe_sort(&mut sorted, |a, b| order(a, b, true) < 0);
                    return BaseValue::List(sorted);
                }
                "unique" => {
                    let mut seen = HashSet::new();
                    return BaseValue::List(
                        items.iter().filter(|i| seen.insert(format!("{}{}", type_name(i), self.display(i)))).cloned().collect(),
                    );
                }
                "slice" => {
                    let bounds: Vec<f64> = values.iter().filter_map(number).collect();
                    return BaseValue::List(items[slice(items.len(), &bounds)].to_vec());
                }
                _ => {}
            },
            BaseValue::File(file) | BaseValue::Link { path: Some(file), .. } => {
                let file = file.clone();
                if let Some(value) = self.file_method(&file, name, &values) {
                    return value;
                }
                if let BaseValue::Link { display: Some(shown), .. } = &receiver {
                    return self.string_method(shown, name, &values);
                }
            }
            BaseValue::Link { target, path: None, .. } => return self.string_method(target, name, &values),
            BaseValue::Object(entries) => match name {
                "keys" => return BaseValue::List(entries.iter().map(|(k, _)| BaseValue::String(k.clone())).collect()),
                "values" => return BaseValue::List(entries.iter().map(|(_, v)| v.clone()).collect()),
                _ => {}
            },
            BaseValue::Regex(regex, _) if name == "matches" => return BaseValue::Bool(regex.is_match(&self.display(&first))),
            _ => {}
        }
        self.note(format!("Unknown method .{name}() on {}", type_name(&receiver)));
        BaseValue::Null
    }

    fn string_method(&mut self, text: &str, name: &str, values: &[BaseValue]) -> BaseValue {
        let first = values.first().cloned().unwrap_or(BaseValue::Null);
        let strings: Vec<String> = values.iter().map(|v| self.display(v)).collect();
        let shown = self.display(&first);
        let string = |s: String| BaseValue::String(s);
        match name {
            "contains" => BaseValue::Bool(text.contains(shown.as_str())),
            "containsAll" => BaseValue::Bool(strings.iter().all(|s| text.contains(s.as_str()))),
            "containsAny" => BaseValue::Bool(strings.iter().any(|s| text.contains(s.as_str()))),
            "startsWith" => BaseValue::Bool(text.starts_with(shown.as_str())),
            "endsWith" => BaseValue::Bool(text.ends_with(shown.as_str())),
            "lower" => string(text.to_lowercase()),
            "upper" => string(text.to_uppercase()),
            "trim" => string(text.trim().to_string()),
            "reverse" => string(text.chars().rev().collect()),
            "repeat" => {
                let count = number(&first).and_then(whole).unwrap_or(0).max(0) as usize;
                // A count too large for memory is bad input, not a crash.
                if count > 10_000_000 / text.len().max(1) {
                    return BaseValue::Null;
                }
                string(text.repeat(count))
            }
            "title" => {
                let mut out = String::new();
                let mut start = true;
                for c in text.chars() {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.push(c);
                    }
                    start = c.is_whitespace();
                }
                string(out)
            }
            "slice" => {
                let chars: Vec<char> = text.chars().collect();
                let bounds: Vec<f64> = values.iter().filter_map(number).collect();
                string(chars[slice(chars.len(), &bounds)].iter().collect())
            }
            "split" => {
                let mut parts: Vec<String> = match &first {
                    BaseValue::Regex(regex, _) => regex.split(text).map(String::from).collect(),
                    _ if shown.is_empty() => text.chars().map(String::from).collect(),
                    _ => text.split(shown.as_str()).map(String::from).collect(),
                };
                if let Some(limit) = values.get(1).and_then(number).and_then(whole)
                    && limit >= 0
                {
                    parts.truncate(limit as usize);
                }
                BaseValue::List(parts.into_iter().map(BaseValue::String).collect())
            }
            "replace" => {
                let replacement = values.get(1).map(|v| self.display(v)).unwrap_or_default();
                match &first {
                    BaseValue::Regex(regex, true) => string(regex.replace_all(text, replacement.as_str()).into_owned()),
                    BaseValue::Regex(regex, false) => string(regex.replace(text, replacement.as_str()).into_owned()),
                    _ => string(text.replace(shown.as_str(), &replacement)),
                }
            }
            _ => {
                self.note(format!("Unknown method .{name}() on string"));
                BaseValue::Null
            }
        }
    }

    fn file_method(&self, path: &str, name: &str, values: &[BaseValue]) -> Option<BaseValue> {
        let first = values.first().cloned().unwrap_or(BaseValue::Null);
        let note = self.index.notes.get(path);
        Some(match name {
            "inFolder" => {
                let folder = self.display(&first);
                let folder = folder.trim_matches('/');
                BaseValue::Bool(folder.is_empty() || path.starts_with(&format!("{folder}/")))
            }
            "hasProperty" => {
                let key = self.display(&first);
                BaseValue::Bool(note.is_some_and(|n| n.parsed.properties.iter().any(|(k, _)| *k == key)))
            }
            "hasTag" => {
                let tags: Vec<String> = note.map(|n| n.parsed.tags.iter().map(|t| t.to_lowercase()).collect()).unwrap_or_default();
                BaseValue::Bool(values.iter().any(|v| {
                    let wanted = self.display(v).to_lowercase();
                    let wanted = wanted.trim_matches('#');
                    tags.iter().any(|t| t == wanted || t.starts_with(&format!("{wanted}/")))
                }))
            }
            "hasLink" | "linksTo" => {
                let Some(target) = self.target(&first) else { return Some(BaseValue::Bool(false)) };
                BaseValue::Bool(note.is_some_and(|n| {
                    n.parsed.links.iter().any(|l| self.index.resolve(&l.target, path).as_deref() == Some(target.as_str()))
                }))
            }
            "asLink" => BaseValue::Link {
                target: path.into(),
                path: Some(path.into()),
                display: if values.is_empty() { None } else { Some(self.display(&first)) },
            },
            "asFile" => BaseValue::File(path.into()),
            _ => return None,
        })
    }

    /// The vault path a file, a link or a name stands for.
    fn target(&self, value: &BaseValue) -> Option<String> {
        match value {
            BaseValue::File(path) => Some(path.clone()),
            BaseValue::Link { path, .. } => path.clone(),
            BaseValue::String(text) => self.resolve(text),
            _ => None,
        }
    }

    // Operators

    fn binary(&self, symbol: &str, left: &BaseValue, right: &BaseValue) -> BaseValue {
        match symbol {
            "==" => BaseValue::Bool(equal(left, right)),
            "!=" => BaseValue::Bool(!equal(left, right)),
            "<" | ">" | "<=" | ">=" => {
                let Some(order) = compare(left, right) else { return BaseValue::Bool(false) };
                BaseValue::Bool(match symbol {
                    "<" => order < 0,
                    ">" => order > 0,
                    "<=" => order <= 0,
                    _ => order >= 0,
                })
            }
            _ => self.arithmetic(symbol, left, right),
        }
    }

    fn arithmetic(&self, symbol: &str, left: &BaseValue, right: &BaseValue) -> BaseValue {
        match (left, right, symbol) {
            (BaseValue::Date(date, time), _, "+" | "-") => {
                if let BaseValue::Date(other, _) = right
                    && symbol == "-"
                {
                    return BaseValue::Duration(BaseSpan::new(0, (*date - *other).num_milliseconds() as f64));
                }
                let Some(mut span) = span(right) else {
                    return if symbol == "+" { BaseValue::String(self.display(left) + &self.display(right)) } else { BaseValue::Null };
                };
                if symbol == "-" {
                    span = span.negated();
                }
                BaseValue::Date(add(span, *date), *time || span.milliseconds % 86_400_000.0 != 0.0)
            }
            (BaseValue::Duration(a), BaseValue::Duration(b), "+") => BaseValue::Duration(a.plus(*b)),
            (BaseValue::Duration(a), BaseValue::Duration(b), "-") => BaseValue::Duration(a.plus(b.negated())),
            (BaseValue::Duration(span), BaseValue::Number(n), "*") => match whole(span.months as f64 * n) {
                Some(months) => BaseValue::Duration(BaseSpan::new(months, span.milliseconds * n)),
                None => BaseValue::Null,
            },
            (BaseValue::Duration(span), BaseValue::Number(n), "/") => BaseValue::Number(span.approximate_milliseconds() / n),
            (BaseValue::String(_), _, "+") | (_, BaseValue::String(_), "+") => BaseValue::String(self.display(left) + &self.display(right)),
            _ => {
                let (Some(a), Some(b)) = (number(left), number(right)) else { return BaseValue::Null };
                match symbol {
                    "+" => BaseValue::Number(a + b),
                    "-" => BaseValue::Number(a - b),
                    "*" => BaseValue::Number(a * b),
                    "/" if b != 0.0 => BaseValue::Number(a / b),
                    "%" if b != 0.0 => BaseValue::Number(a % b),
                    _ => BaseValue::Null,
                }
            }
        }
    }

    // Text

    /// A value as text: in a cell, in `+` with a string, in `join`.
    pub fn display(&self, value: &BaseValue) -> String {
        match value {
            BaseValue::Null => String::new(),
            BaseValue::Bool(flag) => flag.to_string(),
            BaseValue::Number(n) => display_number(*n),
            BaseValue::String(text) => text.clone(),
            BaseValue::Date(date, time) => format_date(*date, if *time { "YYYY-MM-DD HH:mm" } else { "YYYY-MM-DD" }),
            BaseValue::Duration(span) => humanize(span.approximate_milliseconds() / 1000.0),
            BaseValue::List(items) => items.iter().map(|i| self.display(i)).collect::<Vec<_>>().join(", "),
            BaseValue::Link { target, path, display } => {
                display.clone().or_else(|| path.as_deref().map(title)).unwrap_or_else(|| target.clone())
            }
            BaseValue::File(path) => title(path),
            BaseValue::Object(entries) => entries.iter().map(|(k, v)| format!("{k}: {}", self.display(v))).collect::<Vec<_>>().join(", "),
            BaseValue::Regex(regex, _) => format!("/{}/", regex.as_str()),
        }
    }
}

/// A column's heading when the base gives it no `displayName`.
pub fn label(id: &str) -> String {
    if let Some(rest) = id.strip_prefix("file.") {
        return format!("file {rest}");
    }
    id.strip_prefix("formula.").or_else(|| id.strip_prefix("note.")).unwrap_or(id).to_string()
}

/// A property id from `order`, `sort` or `groupBy` -- `file.name`,
/// `formula.x`, `note.x` or a bare name -- as an expression. An id is a
/// path, never parsed as one: `note.date-created` is not a subtraction.
fn column(id: &str) -> BaseExpr {
    if id == "file" {
        return BaseExpr::Name("file".into());
    }
    for space in ["file", "formula", "note"] {
        if let Some(rest) = id.strip_prefix(space).and_then(|r| r.strip_prefix('.')) {
            return BaseExpr::Member(Box::new(BaseExpr::Name(space.into())), rest.into());
        }
    }
    BaseExpr::Index(Box::new(BaseExpr::Name("note".into())), Box::new(BaseExpr::Value(BaseValue::String(id.into()))))
}

fn title(path: &str) -> String {
    if Vault::is_note(path) { Vault::title(path) } else { path.rsplit('/').next().unwrap_or("").to_string() }
}

/// A sort that never panics on an order that is not total, as a mix of
/// kinds can make: a merge sort, stable.
fn safe_sort<T: Clone>(items: &mut [T], less: impl Fn(&T, &T) -> bool) {
    fn merge<T: Clone>(items: &mut [T], less: &impl Fn(&T, &T) -> bool) {
        if items.len() < 2 {
            return;
        }
        let middle = items.len() / 2;
        merge(&mut items[..middle], less);
        merge(&mut items[middle..], less);
        let (left, right) = (items[..middle].to_vec(), items[middle..].to_vec());
        let (mut i, mut j) = (0, 0);
        for slot in items.iter_mut() {
            if j >= right.len() || (i < left.len() && !less(&right[j], &left[i])) {
                *slot = left[i].clone();
                i += 1;
            } else {
                *slot = right[j].clone();
                j += 1;
            }
        }
    }
    merge(items, &less);
}

/// `2026-10-25`, `2026-10-25T09:30`, `2026-10-25 09:30:00`, in the local
/// time zone unless it ends in `Z`.
pub fn parse_date(text: &str) -> Option<(DateTime<Local>, bool)> {
    let caps = DATE.captures(text.trim_matches([' ', '\t']))?;
    let int = |i: usize| caps.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
    let day = NaiveDate::from_ymd_opt(int(1)? as i32, int(2)?, int(3)?)?;
    let has_time = caps.get(4).is_some();
    let at = day.and_hms_opt(int(4).unwrap_or(0), int(5).unwrap_or(0), int(6).unwrap_or(0))?;
    let date =
        if caps.get(7).is_some() { Utc.from_utc_datetime(&at).with_timezone(&Local) } else { Local.from_local_datetime(&at).earliest()? };
    Some((date, has_time))
}

fn start_of_day(date: DateTime<Local>) -> DateTime<Local> {
    date.date_naive().and_hms_opt(0, 0, 0).and_then(|d| Local.from_local_datetime(&d).earliest()).unwrap_or(date)
}

fn span(value: &BaseValue) -> Option<BaseSpan> {
    match value {
        BaseValue::Duration(span) => Some(*span),
        BaseValue::String(text) => BaseSpan::parse(text),
        BaseValue::Number(ms) => Some(BaseSpan::new(0, *ms)),
        _ => None,
    }
}

fn add(span: BaseSpan, date: DateTime<Local>) -> DateTime<Local> {
    let months = u32::try_from(span.months.unsigned_abs()).ok().map(Months::new);
    let shifted = match months {
        _ if span.months == 0 => Some(date),
        Some(m) if span.months > 0 => date.checked_add_months(m),
        Some(m) => date.checked_sub_months(m),
        None => None,
    }
    .unwrap_or(date);
    match whole(span.milliseconds).and_then(TimeDelta::try_milliseconds) {
        Some(delta) => shifted.checked_add_signed(delta).unwrap_or(shifted),
        None => shifted,
    }
}

fn number(value: &BaseValue) -> Option<f64> {
    match value {
        BaseValue::Number(n) => Some(*n),
        BaseValue::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        BaseValue::String(text) => text.trim_matches([' ', '\t']).parse().ok(),
        BaseValue::Date(date, _) => Some(date.timestamp_millis() as f64),
        BaseValue::Duration(span) => Some(span.approximate_milliseconds()),
        _ => None,
    }
}

pub fn truthy(value: &BaseValue) -> bool {
    match value {
        BaseValue::Null => false,
        BaseValue::Bool(flag) => *flag,
        BaseValue::Number(n) => *n != 0.0 && !n.is_nan(),
        BaseValue::String(text) => !text.is_empty(),
        BaseValue::List(items) => !items.is_empty(),
        _ => true,
    }
}

pub fn is_empty(value: &BaseValue) -> bool {
    match value {
        BaseValue::Null => true,
        BaseValue::String(text) => text.is_empty(),
        BaseValue::List(items) => items.is_empty(),
        BaseValue::Object(entries) => entries.is_empty(),
        _ => false,
    }
}

pub fn type_name(value: &BaseValue) -> &'static str {
    match value {
        BaseValue::Null => "null",
        BaseValue::Bool(_) => "boolean",
        BaseValue::Number(_) => "number",
        BaseValue::String(_) => "string",
        BaseValue::Date(..) => "date",
        BaseValue::Duration(_) => "duration",
        BaseValue::List(_) => "list",
        BaseValue::Link { .. } => "link",
        BaseValue::File(_) => "file",
        BaseValue::Object(_) => "object",
        BaseValue::Regex(..) => "regexp",
    }
}

pub fn equal(a: &BaseValue, b: &BaseValue) -> bool {
    use BaseValue as V;
    match (a, b) {
        (V::Null, V::Null) => true,
        (V::Bool(x), V::Bool(y)) => x == y,
        (V::Number(x), V::Number(y)) => x == y,
        (V::String(x), V::String(y)) => x == y,
        (V::Date(..), V::Date(..)) | (V::Date(..), V::String(_)) | (V::String(_), V::Date(..)) | (V::Duration(_), V::Duration(_)) => {
            compare(a, b) == Some(0)
        }
        (V::List(x), V::List(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| equal(p, q)),
        (V::File(x), V::File(y)) => x == y,
        (V::Link { path: Some(x), .. }, V::File(y)) | (V::File(y), V::Link { path: Some(x), .. }) => x == y,
        (V::Link { target: t, path: x, .. }, V::Link { target: u, path: y, .. }) => {
            if x.is_some() {
                x == y
            } else {
                t == u
            }
        }
        (V::Link { target, path, .. }, V::String(text)) | (V::String(text), V::Link { target, path, .. }) => {
            text == target || path.as_ref() == Some(text) || path.as_deref().is_some_and(|p| Vault::title(p) == *text)
        }
        (V::File(path), V::String(text)) | (V::String(text), V::File(path)) => text == path || Vault::title(path) == *text,
        _ => false,
    }
}

/// Negative, zero or positive; `None` when the two cannot be ordered.
pub fn compare(a: &BaseValue, b: &BaseValue) -> Option<i32> {
    use BaseValue as V;
    let sign = |x: f64, y: f64| {
        if x < y {
            -1
        } else if x > y {
            1
        } else {
            0
        }
    };
    match (a, b) {
        (V::Number(x), V::Number(y)) => Some(sign(*x, *y)),
        (V::Date(x, _), V::Date(y, _)) => Some(sign(x.timestamp_millis() as f64, y.timestamp_millis() as f64)),
        (V::Date(..), V::String(text)) => parse_date(text).and_then(|(d, t)| compare(a, &V::Date(d, t))),
        (V::String(text), V::Date(..)) => parse_date(text).and_then(|(d, t)| compare(&V::Date(d, t), b)),
        (V::Duration(x), V::Duration(y)) => Some(sign(x.approximate_milliseconds(), y.approximate_milliseconds())),
        (V::Duration(x), V::Number(y)) => Some(sign(x.approximate_milliseconds(), *y)),
        (V::Number(x), V::Duration(y)) => Some(sign(*x, y.approximate_milliseconds())),
        (V::String(x), V::String(y)) => Some(match standard_compare(x, y) {
            Ordering::Less => -1,
            Ordering::Greater => 1,
            Ordering::Equal => 0,
        }),
        (V::Bool(x), V::Bool(y)) => Some(if x == y {
            0
        } else if *x {
            1
        } else {
            -1
        }),
        (V::File(x), V::File(y)) => compare(&V::String(Vault::title(x)), &V::String(Vault::title(y))),
        (V::Link { target: t, display: s, .. }, V::Link { target: u, display: r, .. }) => {
            compare(&V::String(s.clone().unwrap_or_else(|| t.clone())), &V::String(r.clone().unwrap_or_else(|| u.clone())))
        }
        (V::List(x), V::List(y)) => {
            for (p, q) in x.iter().zip(y) {
                if let Some(order) = compare(p, q)
                    && order != 0
                {
                    return Some(order);
                }
            }
            Some(sign(x.len() as f64, y.len() as f64))
        }
        _ => None,
    }
}

/// The order of two rows for a sort key: empty values last both ways,
/// and values of different kinds by their text.
pub fn order(a: &BaseValue, b: &BaseValue, ascending: bool) -> i32 {
    let (empty_a, empty_b) = (is_empty(a), is_empty(b));
    if empty_a || empty_b {
        return if empty_a == empty_b {
            0
        } else if empty_a {
            1
        } else {
            -1
        };
    }
    let order = compare(a, b).or_else(|| compare(&BaseValue::String(plain(a)), &BaseValue::String(plain(b)))).unwrap_or(0);
    if ascending { order } else { -order }
}

fn plain(value: &BaseValue) -> String {
    match value {
        BaseValue::String(text) => text.clone(),
        BaseValue::Number(n) => format!("{n:?}"),
        _ => type_name(value).to_string(),
    }
}

fn slice(count: usize, bounds: &[f64]) -> std::ops::Range<usize> {
    let n = count as f64;
    let clamp = |value: f64| -> usize {
        let at = if value.is_nan() { 0.0 } else { value.clamp(-n, n).trunc() };
        let at = if at < 0.0 { n + at } else { at };
        (at.max(0.0) as usize).min(count)
    };
    let start = bounds.first().map_or(0, |&b| clamp(b));
    let end = bounds.get(1).map_or(count, |&b| clamp(b));
    start..end.max(start)
}

fn display_number(n: f64) -> String {
    if n == n.round() && n.abs() < 1e15 {
        return (n as i64).to_string();
    }
    if !n.is_finite() {
        return if n.is_nan() {
            "nan".into()
        } else if n > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let mut text = format!("{n:.4}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// moment's `fromNow`: "3 days ago", "in an hour".
pub fn relative(date: DateTime<Local>, now: DateTime<Local>) -> String {
    let seconds = (date - now).num_milliseconds() as f64 / 1000.0;
    let text = humanize(seconds.abs());
    if seconds > 0.0 { format!("in {text}") } else { format!("{text} ago") }
}

/// moment's thresholds for a length of time in words.
pub fn humanize(seconds: f64) -> String {
    let s = seconds.abs();
    let (minutes, hours) = (s / 60.0, s / 3600.0);
    let days = hours / 24.0;
    let count = |value: f64, unit: &str| format!("{} {unit}", whole(value.round()).map_or("many".to_string(), |w| w.to_string()));
    if s < 45.0 {
        "a few seconds".into()
    } else if s < 90.0 {
        "a minute".into()
    } else if minutes < 45.0 {
        count(minutes, "minutes")
    } else if minutes < 90.0 {
        "an hour".into()
    } else if hours < 22.0 {
        count(hours, "hours")
    } else if hours < 36.0 {
        "a day".into()
    } else if days < 26.0 {
        count(days, "days")
    } else if days < 45.0 {
        "a month".into()
    } else if days < 320.0 {
        count(days / 30.4375, "months")
    } else if days < 548.0 {
        "a year".into()
    } else {
        count(days / 365.25, "years")
    }
}

/// A date in a moment.js format, such as `ddd D MMM, HH:mm`.
pub fn format_date(date: DateTime<Local>, pattern: &str) -> String {
    let (year, month, day) = (date.year(), date.month() as usize, date.day());
    let (hour, minute, second) = (date.hour(), date.minute(), date.second());
    let weekday = date.weekday().num_days_from_sunday() as usize;
    let two = |v: u32| format!("{v:02}");
    let ordinal = |v: u32| {
        let suffix = if (11..=13).contains(&(v % 100)) {
            "th"
        } else {
            match v % 10 {
                1 => "st",
                2 => "nd",
                3 => "rd",
                _ => "th",
            }
        };
        format!("{v}{suffix}")
    };
    let offset = date.offset().local_minus_utc() / 60;
    let zone = format!("{}{}", if offset < 0 { "-" } else { "+" }, two(offset.unsigned_abs() / 60));
    let twelve = if hour % 12 == 0 { 12 } else { hour % 12 };
    FORMAT_TOKEN
        .replace_all(pattern, |caps: &regex::Captures| {
            let token = &caps[0];
            match token {
                "YYYY" => year.to_string(),
                "YY" => two(year.rem_euclid(100) as u32),
                "Q" => ((month - 1) / 3 + 1).to_string(),
                "MMMM" => MONTHS[month - 1].to_string(),
                "MMM" => MONTHS[month - 1][..3].to_string(),
                "MM" => two(month as u32),
                "M" => month.to_string(),
                "Do" => ordinal(day),
                "DD" => two(day),
                "D" => day.to_string(),
                "dddd" => WEEKDAYS[weekday].to_string(),
                "ddd" => WEEKDAYS[weekday][..3].to_string(),
                "dd" => WEEKDAYS[weekday][..2].to_string(),
                "d" => weekday.to_string(),
                "HH" => two(hour),
                "H" => hour.to_string(),
                "hh" => two(twelve),
                "h" => twelve.to_string(),
                "mm" => two(minute),
                "m" => minute.to_string(),
                "ss" => two(second),
                "s" => second.to_string(),
                "SSS" => format!("{:03}", date.nanosecond() / 1_000_000),
                "A" => (if hour < 12 { "AM" } else { "PM" }).to_string(),
                "a" => (if hour < 12 { "am" } else { "pm" }).to_string(),
                "ZZ" => format!("{zone}{}", two(offset.unsigned_abs() % 60)),
                "Z" => format!("{zone}:{}", two(offset.unsigned_abs() % 60)),
                "X" => date.timestamp().to_string(),
                "x" => date.timestamp_millis().to_string(),
                _ => token[1..token.len() - 1].to_string(),
            }
        })
        .into_owned()
}

/// A number in [0, 1) with no crate for it: the clock's nanoseconds, mixed.
fn random() -> f64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut x = nanos ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note_parser::NoteParser;
    use crate::vault::NoteEntry;

    // Every fixture here is made up. The repository is public; the vault is not.

    /// 2026-10-25 12:00 local time.
    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 25, 12, 0, 0).unwrap()
    }

    fn make_index() -> VaultIndex {
        let mut index = VaultIndex::default();
        let notes = [
            (
                "Shops/Bakery/Bakery.md",
                "---\nstage: open\nopened: 2026-10-20\nnext: 2026-10-27T09:30\nroles:\n  - baker\n---\nSee [[Cafe]].",
                2,
            ),
            ("Shops/Bakery/Ovens.md", "Ovens of [[Bakery]] #kit/ovens", 40),
            ("Shops/Cafe/Cafe.md", "---\nstage: closed\nopened: 2026-09-01\nclosed: 2026-10-11\nrating: 4.5\n---\n", 10),
            ("Shops/CLAUDE.md", "rules", 1),
            ("Home.md", "[[Bakery]] and [[Cafe]]", 100),
        ];
        for (path, text, days_old) in notes {
            let modified = (now() - TimeDelta::days(days_old)).into();
            index.notes.insert(path.into(), NoteEntry { path: path.into(), modified, parsed: NoteParser::parse(text) });
            index.files.insert(path.into());
        }
        index.files.insert("uploads/map.png".into());
        index.derive();
        index
    }

    fn types() -> HashMap<String, String> {
        HashMap::from([("roles".into(), "multitext".into()), ("next".into(), "datetime".into())])
    }

    fn evaluate_in(source: &str, path: &str) -> String {
        let index = make_index();
        let mut engine = BaseEngine::new(&index, now(), types(), |_| None);
        let expression = BaseParser::parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        let value = engine.evaluate(&expression, path, &Locals::new());
        engine.display(&value)
    }

    fn evaluate(source: &str) -> String {
        evaluate_in(source, "Shops/Bakery/Bakery.md")
    }

    #[test]
    fn file_properties() {
        assert_eq!(evaluate("file.name"), "Bakery.md");
        assert_eq!(evaluate("file.basename"), "Bakery");
        assert_eq!(evaluate("file.ext"), "md");
        assert_eq!(evaluate("file.folder"), "Shops/Bakery");
        assert_eq!(evaluate("file.folder.split(\"/\")[1]"), "Bakery");
        assert_eq!(evaluate("file.backlinks.length"), "2");
        assert_eq!(evaluate("file.inFolder(\"Shops\") && !file.inFolder(\"Shop\")"), "true");
        assert_eq!(evaluate("file.hasProperty(\"stage\")"), "true");
        assert_eq!(evaluate_in("file.hasTag(\"kit\")", "Shops/Bakery/Ovens.md"), "true");
        assert_eq!(evaluate("file.hasLink(\"Cafe\")"), "true");
        assert_eq!(evaluate_in("file.name", "Home.md"), "Home.md");
        assert_eq!(evaluate_in("file.folder", "Home.md"), "/");
    }

    #[test]
    fn properties_and_operators() {
        let cafe = "Shops/Cafe/Cafe.md";
        assert_eq!(evaluate("stage == \"open\""), "true");
        assert_eq!(evaluate("note.stage"), "open");
        assert_eq!(evaluate("roles.contains(\"baker\")"), "true");
        assert_eq!(evaluate("[\"open\", \"new\"].contains(stage)"), "true");
        assert_eq!(evaluate_in("rating * 2", cafe), "9");
        assert_eq!(evaluate_in("(closed - opened) / 86400000", cafe), "40");
        assert_eq!(evaluate_in("((closed - opened) / 86400000).round()", cafe), "40");
        assert_eq!(evaluate("missing"), "");
        assert_eq!(evaluate("missing.lower()"), "");
        assert_eq!(evaluate("if(missing, \"yes\", \"no\")"), "no");
        assert_eq!(evaluate("if(stage == \"open\", \"1 Open\", if(stage == \"closed\", \"2 Closed\", \"3\"))"), "1 Open");
        assert_eq!(evaluate("\"a\" + 1 + 2"), "a12");
        assert_eq!(evaluate("1 + 2 * 3 - -1"), "8");
        assert_eq!(evaluate("10 % 4"), "2");
    }

    #[test]
    fn dates() {
        assert_eq!(evaluate("next >= today()"), "true");
        assert_eq!(evaluate("opened < today() - \"3 days\""), "true");
        assert_eq!(evaluate("file.mtime > now() - \"30 days\""), "true");
        assert_eq!(evaluate_in("file.mtime > now() - \"30 days\"", "Shops/Bakery/Ovens.md"), "false");
        assert_eq!(evaluate("next.format(\"ddd D MMM, HH:mm\")"), "Tue 27 Oct, 09:30");
        assert_eq!(evaluate("next.format(\"YYYY-MM-DD [at] h A, Do\")"), "2026-10-27 at 9 AM, 27th");
        assert_eq!(evaluate("next.relative()"), "in 2 days");
        assert_eq!(evaluate("opened.relative()"), "6 days ago");
        assert_eq!(evaluate_in("file.mtime.relative()", "Home.md"), "3 months ago");
        assert_eq!(evaluate("date(\"2024-12-01\") + \"1M\" + \"4h\""), "2025-01-01 04:00");
        assert_eq!(evaluate("opened"), "2026-10-20");
        assert_eq!(evaluate("opened.year"), "2026");
    }

    #[test]
    fn strings_lists_and_regex() {
        assert_eq!(evaluate("\"a-b-c\".split(\"-\", 2).join(\"+\")"), "a+b");
        assert_eq!(evaluate("\"Hello world\".replace(/o/g, \"0\")"), "Hell0 w0rld");
        assert_eq!(evaluate("\"hello big world\".title()"), "Hello Big World");
        assert_eq!(evaluate("\"abcdef\".slice(1, -1)"), "bcde");
        assert_eq!(evaluate("[3, 1, 2].sort().reverse()"), "3, 2, 1");
        assert_eq!(evaluate("[1, 2, 3].filter(value > 1).map(value * 10)"), "20, 30");
        assert_eq!(evaluate("[1, 2, 3].reduce(acc + value, 0)"), "6");
        assert_eq!(evaluate("[1, 1, 2].unique().length"), "2");
        assert_eq!(evaluate("/ak/.matches(file.name)"), "true");
        assert_eq!(evaluate("(5).toFixed(2)"), "5.00");
    }

    const SOURCE: &str = r#"filters:
  and:
    - file.inFolder("Shops")
    - file.ext == "md"
    - file.basename != "CLAUDE"
formulas:
  shop: file.folder.split("/")[1]
  label: if(stage == "open", "1 Open", "2 " + formula.shop)
  loop: formula.loop + 1
properties:
  formula.shop:
    displayName: Shop
views:
  - type: table
    name: Shops
    filters:
      and:
        - file.basename == formula.shop
    order:
      - file.name
      - formula.shop
      - rating
    sort:
      - property: file.mtime
        direction: DESC
  - type: table
    name: Grouped
    groupBy:
      property: formula.shop
      direction: DESC
    order:
      - file.name
      - formula.label
      - roles
    sort:
      - property: file.name
        direction: ASC
    limit: 2
  - type: cards
    name: Broken
    filters:
      or:
        - stage ==
        - formula.loop > 0
"#;

    #[test]
    fn runs_the_views() {
        let index = make_index();
        let mut engine = BaseEngine::new(&index, now(), types(), |_| None);
        let tables = engine.run(&Base::parse(SOURCE), "Shops/Shops.base");
        assert_eq!(tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Shops", "Grouped", "Broken"]);

        let shops = &tables[0];
        assert_eq!(shops.columns, ["file name", "Shop", "rating"]);
        assert!(shops.groups.len() == 1 && shops.groups[0].name.is_none());
        // Newest first: the bakery changed two days ago, the cafe ten.
        assert_eq!(
            shops.groups[0].rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["Shops/Bakery/Bakery.md", "Shops/Cafe/Cafe.md"]
        );
        let cafe = &shops.groups[0].rows[1].cells;
        assert_eq!(
            cafe[0],
            BaseCell { parts: vec![CellPart { text: "Cafe".into(), path: Some("Shops/Cafe/Cafe.md".into()) }], ..BaseCell::default() }
        );
        assert_eq!(cafe[1].text(), "Cafe");
        assert_eq!(cafe[2], BaseCell { parts: vec![CellPart::text("4.5")], numeric: true, ..BaseCell::default() });

        // The limit keeps the first two by name, then the groups sort Z to A.
        let grouped = &tables[1];
        assert_eq!(grouped.count, 2);
        assert_eq!(grouped.groups.iter().map(|g| g.name.clone().unwrap()).collect::<Vec<_>>(), ["Cafe", "Bakery"]);
        assert_eq!(grouped.groups.iter().map(|g| g.rows[0].cells[1].text()).collect::<Vec<_>>(), ["2 Cafe", "1 Open"]);
        assert_eq!(grouped.groups[1].rows[0].cells[2].text(), "baker");

        // A statement that does not parse is not applied, and is reported;
        // so is a formula that uses itself.
        assert_eq!(tables[2].kind, "cards");
        assert!(engine.problems.iter().any(|p| p.contains("stage ==")), "{:?}", engine.problems);
        assert!(engine.problems.iter().any(|p| p.contains("loop uses itself")), "{:?}", engine.problems);
    }

    #[test]
    fn cards_and_list_options() {
        let base = Base::parse(
            "views:\n  - type: cards\n    name: Gallery\n    image: note.cover\n    imageFit: contain\n    imageAspectRatio: 0.75\n    cardSize: 260\n  - type: list\n    name: Lines\n    markers: number\n    indentProperties: true\n    separator: \"|\"\n  - type: cards\n    name: Odd\n    imageFit: stretch\n    imageAspectRatio: -1\n    cardSize: wide\n  - type: kanban\n    name: Plugin\n",
        );
        assert_eq!(base.views.iter().map(BaseView::draws_as).collect::<Vec<_>>(), ["cards", "list", "cards", "table"]);
        let cards = &base.views[0].layout;
        assert_eq!(cards.image.as_deref(), Some("note.cover"));
        assert_eq!((cards.image_fit.as_str(), cards.image_aspect_ratio, cards.card_size), ("contain", 0.75, 260.0));
        let list = &base.views[1].layout;
        assert!(list.markers == "number" && list.indent_properties && list.separator == "|");
        // A value the app cannot read keeps the default.
        assert_eq!(base.views[2].layout, BaseLayout::default());
        assert_eq!(base.views[3].kind, "kanban");
    }

    #[test]
    fn a_cards_cover_is_a_picture_a_web_address_or_a_colour() {
        let mut index = VaultIndex::default();
        let notes = [
            ("A.md", "---\ncover: \"[[map.png]]\"\n---\n"),
            ("B.md", "---\ncover: https://example.com/b.jpg\n---\n"),
            ("C.md", "---\ncover: \"#ff5070\"\n---\n"),
            ("D.md", "---\ncover: \"[[A]]\"\n---\n"),
            ("E.md", "---\ncover: uploads/map.png\n---\n"),
        ];
        for (path, text) in notes {
            let modified = now().into();
            index.notes.insert(path.into(), NoteEntry { path: path.into(), modified, parsed: NoteParser::parse(text) });
            index.files.insert(path.into());
        }
        index.files.insert("uploads/map.png".into());
        index.derive();
        let base = Base::parse(
            "filters: file.ext == \"md\"\nviews:\n  - type: cards\n    name: C\n    image: note.cover\n  - type: table\n    name: T\n    image: note.cover\n",
        );
        let tables = BaseEngine::new(&index, now(), HashMap::new(), |_| None).run(&base, "");
        let covers: Vec<Option<BaseImage>> = tables[0].groups[0].rows.iter().map(|r| r.image.clone()).collect();
        assert_eq!(tables[0].draws_as, "cards");
        assert_eq!(
            covers,
            [
                Some(BaseImage::File("uploads/map.png".into())),
                Some(BaseImage::Url("https://example.com/b.jpg".into())),
                Some(BaseImage::Color("#ff5070".into())),
                None,
                Some(BaseImage::File("uploads/map.png".into())),
            ]
        );
        // A table has no covers.
        assert!(tables[1].groups[0].rows.iter().all(|r| r.image.is_none()));
    }

    #[test]
    fn a_base_with_no_views_has_one() {
        let index = make_index();
        let tables = BaseEngine::new(&index, now(), types(), |_| None).run(&Base::parse("filters: file.name == \"Home.md\""), "");
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].groups[0].rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), ["Home.md"]);
    }

    #[test]
    fn a_column_id_is_a_property_not_a_sum() {
        let mut index = VaultIndex::default();
        for (path, text) in
            [("A.md", "---\ndate-created: 2026-10-01\nmy-key: b\n---\n"), ("B.md", "---\ndate-created: 2026-09-01\nmy-key: a\n---\n")]
        {
            index.notes.insert(path.into(), NoteEntry { path: path.into(), modified: now().into(), parsed: NoteParser::parse(text) });
            index.files.insert(path.into());
        }
        index.derive();
        let base = Base::parse(
            "views:\n  - type: table\n    name: All\n    order:\n      - file.name\n      - note.date-created\n      - my-key\n    sort:\n      - property: my-key\n        direction: ASC\n",
        );
        let table = BaseEngine::new(&index, now(), HashMap::new(), |_| None).run(&base, "").remove(0);
        assert_eq!(table.groups[0].rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), ["B.md", "A.md"]);
        assert_eq!(table.groups[0].rows[0].cells.iter().map(BaseCell::text).collect::<Vec<_>>(), ["B", "2026-09-01", "a"]);
    }

    #[test]
    fn bad_numbers_are_null_not_a_crash() {
        for source in [
            "[1, 2][0 / 0]",
            "\"ab\"[1 / 0]",
            "[1, 2][100000000000000000000000]",
            "(1.5).round(1 / 0)",
            "(1.5).toFixed(0 / 0)",
            "\"ab\".repeat(1 / 0)",
            "\"a-b\".split(\"-\", -1)",
            "\"a-b\".split(\"-\", 0 / 0)",
            "duration(\"1M\") * (1 / 0)",
            "\"abc\".slice(0 / 0, 1 / 0)",
            "[1, 2, 3].slice(-100000000000000000000000)",
            "duration(\"99999999999999999999 years\")",
        ] {
            evaluate(source);
        }
        assert_eq!(evaluate("[1, 2][0 / 0]"), "");
        assert_eq!(evaluate("\"a-b\".split(\"-\", -1).length"), "2");
        assert_eq!(evaluate("\"abc\".slice(0 / 0, 1 / 0)"), "abc");
    }

    /// Runs every base of the vault in `DAFTAR_BASES_VAULT`, reading only, and
    /// prints what each view finds and the problems it reports.
    #[test]
    #[ignore]
    fn the_vaults_bases() {
        let Some(root) = std::env::var_os("DAFTAR_BASES_VAULT") else { return };
        let root = std::path::PathBuf::from(root);
        let index = Vault::index(&root, None);
        let types = BaseEngine::types(&root);
        let mut sources: Vec<(String, String)> = Vec::new();
        for path in &index.files {
            let Ok(text) = std::fs::read_to_string(root.join(path)) else { continue };
            if Vault::is_base(path) {
                sources.push((path.clone(), text));
            } else if Vault::is_note(path) {
                let mut block: Option<Vec<&str>> = None;
                for line in text.lines() {
                    let mut line = line.trim_start();
                    while let Some(rest) = line.strip_prefix('>') {
                        line = rest.trim_start();
                    }
                    match &mut block {
                        None if line.trim() == "```base" => block = Some(Vec::new()),
                        Some(b) if line.trim().starts_with("```") => {
                            sources.push((path.clone(), b.join("\n")));
                            block = None;
                        }
                        Some(b) => b.push(line),
                        None => {}
                    }
                }
            }
        }
        for (path, source) in sources {
            let mut engine = BaseEngine::new(&index, Local::now(), types.clone(), |p| BaseEngine::stat_file(&root, p));
            let tables = engine.run(&Base::parse(&source), &path);
            let counts: Vec<String> = tables.iter().map(|t| format!("{}={}", t.name, t.count)).collect();
            println!("{path}: {}", counts.join(", "));
            for problem in &engine.problems {
                println!("  problem: {problem}");
            }
        }
    }
}
