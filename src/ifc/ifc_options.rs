//! Options on the IFC4 file `rvt-ifc` writes: how much it holds
//! ([`IfcProfile`]), its length unit ([`IfcLengthUnit`]), where its origin is
//! ([`IfcOrigin`]) and which elements it keeps (a category or class list).
//!
//! Each option is applied to the finished STEP text, so the writer and every
//! other output (IFCX, USD, `.frag`, glTF) stay as they are. Revit's own IFC
//! export is the reference for what each option means:
//!
//! - **Profile.** `full` writes everything. `coordination` keeps the IFC
//!   common property sets (`Pset_*`) and leaves out Revit's own parameter sets
//!   and the base quantities, as Revit's coordination setups do ("Export IFC
//!   common property sets" on, "Export Revit property sets" and "Export base
//!   quantities" off). `geometry` leaves out every property set and quantity.
//!   Both keep each element's `Revit` set (its ElementId and category), so an
//!   element can still be traced to the model.
//! - **Units.** Only the length unit changes for `m` and `mm` (areas stay in
//!   square metres and volumes in cubic metres, as Revit's metric exports
//!   write them); `ft` and `in` also write areas and volumes in square and
//!   cubic feet or inches. Every length value, coordinate and length
//!   quantity is rescaled, and `IfcMapConversion.Scale` takes the new unit to
//!   the map's.
//! - **Origin.** `internal` keeps Revit's internal origin (the default).
//!   `project-base` moves the origin to the project base point without a
//!   rotation; `survey-point` moves it to the survey point and turns the axes
//!   to true north, as Revit's export does for its "Project Base Point" and
//!   "Survey Point" coordinate bases. The move is the `IfcSite` placement,
//!   which every other placement is relative to, and the `IfcMapConversion`
//!   is rewritten so the map coordinates of every point stay the same.
//! - **Categories.** Only the elements whose Revit category (their `Revit`
//!   or `Other` set) or IFC class is on the list are kept, with their
//!   openings, ports and aggregated parts. Types no element keeps, and
//!   everything only they used, are left out. An element contained in a
//!   room that is left out moves to the room's storey.

use super::IfcModel;
use crate::georeference::Georeference;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

/// How much of the model the IFC file holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IfcProfile {
    /// Everything the export writes.
    #[default]
    Full,
    /// Geometry, identity and the IFC common property sets.
    Coordination,
    /// Geometry and identity only.
    Geometry,
}

/// The IFC file's length unit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IfcLengthUnit {
    #[default]
    Metre,
    Millimetre,
    Foot,
    Inch,
}

impl IfcLengthUnit {
    /// Lengths in this unit per metre.
    pub fn per_metre(self) -> f64 {
        match self {
            IfcLengthUnit::Metre => 1.0,
            IfcLengthUnit::Millimetre => 1000.0,
            IfcLengthUnit::Foot => 1.0 / 0.3048,
            IfcLengthUnit::Inch => 1.0 / 0.0254,
        }
    }

    /// Areas and volumes follow the length unit for feet and inches; for
    /// millimetres they stay in square and cubic metres.
    fn area_volume_factor(self) -> (f64, f64) {
        match self {
            IfcLengthUnit::Metre | IfcLengthUnit::Millimetre => (1.0, 1.0),
            u => (u.per_metre().powi(2), u.per_metre().powi(3)),
        }
    }
}

/// Where the IFC file's origin is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IfcOrigin {
    /// Revit's internal origin.
    #[default]
    Internal,
    /// The project base point, axes unchanged.
    ProjectBase,
    /// The survey point, axes turned to true north.
    SurveyPoint,
}

/// What [`apply`] does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IfcOptions {
    pub profile: IfcProfile,
    pub units: IfcLengthUnit,
    pub origin: IfcOrigin,
    /// Revit category names (`Walls`) or IFC class names (`IfcWall`), case
    /// insensitive. Empty keeps every element.
    pub categories: Vec<String>,
}

impl IfcOptions {
    /// Whether the options change nothing.
    pub fn is_default(&self) -> bool {
        *self == IfcOptions::default()
    }
}

/// What [`apply`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct IfcOptionsReport {
    /// Property sets and quantity sets left out by the profile.
    pub property_sets_left_out: usize,
    /// Elements left out by the category list (openings, ports and parts
    /// that went with them included).
    pub elements_left_out: usize,
    /// Elements kept by the category list.
    pub elements_kept: usize,
    /// Instances left out because nothing kept referenced them any more.
    pub instances_dropped: usize,
    /// The list entries that matched no element, so a typo shows.
    pub unmatched_categories: Vec<String>,
}

/// Why an origin cannot be moved: the point was not read from the file.
pub fn origin_unavailable(model: &IfcModel, origin: IfcOrigin) -> Option<String> {
    let g = model.georeference.as_ref();
    match origin {
        IfcOrigin::Internal => None,
        IfcOrigin::ProjectBase if g.and_then(|g| g.project_base_point).is_none() => Some(
            "--ifc-origin project-base: the project base point was not read from this file".into(),
        ),
        IfcOrigin::SurveyPoint if g.and_then(|g| g.survey_point).is_none() => {
            Some("--ifc-origin survey-point: the survey point was not read from this file".into())
        }
        _ => None,
    }
}

/// Apply `options` to `step`, an IFC4 file the writer made. `georeference`
/// is the model's; it is needed only to move the origin.
pub fn apply(
    step: &str,
    options: &IfcOptions,
    georeference: Option<&Georeference>,
) -> Result<(String, IfcOptionsReport), String> {
    let mut report = IfcOptionsReport::default();
    if options.is_default() {
        return Ok((step.to_string(), report));
    }
    let mut file = StepFile::parse(step)?;
    file.leave_out(options, &mut report)?;
    if options.origin != IfcOrigin::Internal {
        let g = georeference.ok_or("--ifc-origin: the file's georeference was not read")?;
        file.move_origin(options.origin, g)?;
    }
    if options.units != IfcLengthUnit::Metre {
        file.rescale(options.units)?;
    }
    Ok((file.write(), report))
}

// ---- STEP text ------------------------------------------------------------

/// One `#id=NAME(args);` instance; `args` without the outer parentheses.
struct Instance {
    name: String,
    args: String,
}

/// A parsed attribute.
#[derive(Debug, Clone, PartialEq)]
enum Arg {
    /// A number, string, enumeration, `$` or `*`, as written.
    Atom(String),
    Ref(u32),
    List(Vec<Arg>),
    /// A typed value, `IFCLABEL('x')`.
    Typed(String, Vec<Arg>),
}

impl Arg {
    fn write(&self, out: &mut String) {
        match self {
            Arg::Atom(a) => out.push_str(a),
            Arg::Ref(r) => {
                let _ = write!(out, "#{r}");
            }
            Arg::List(items) => {
                out.push('(');
                write_args(items, out);
                out.push(')');
            }
            Arg::Typed(name, items) => {
                out.push_str(name);
                out.push('(');
                write_args(items, out);
                out.push(')');
            }
        }
    }

    fn target(&self) -> Option<u32> {
        match self {
            Arg::Ref(r) => Some(*r),
            _ => None,
        }
    }

    fn refs(&self) -> Vec<u32> {
        match self {
            Arg::Ref(r) => vec![*r],
            Arg::List(items) => items.iter().filter_map(Arg::target).collect(),
            _ => Vec::new(),
        }
    }

    /// The text of a STEP string atom, its doubled quotes undone.
    fn text(&self) -> Option<String> {
        match self {
            Arg::Atom(a) if a.len() >= 2 && a.starts_with('\'') && a.ends_with('\'') => {
                Some(a[1..a.len() - 1].replace("''", "'"))
            }
            Arg::Typed(_, items) => items.first().and_then(Arg::text),
            _ => None,
        }
    }

    /// Multiply every number inside by `k`.
    fn scale(&mut self, k: f64) {
        match self {
            Arg::Atom(a) => {
                if let Some(v) = parse_real(a) {
                    *a = format_real(v * k);
                }
            }
            Arg::List(items) | Arg::Typed(_, items) => items.iter_mut().for_each(|i| i.scale(k)),
            Arg::Ref(_) => {}
        }
    }
}

fn write_args(items: &[Arg], out: &mut String) {
    for (i, a) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        a.write(out);
    }
}

fn args_string(items: &[Arg]) -> String {
    let mut s = String::new();
    write_args(items, &mut s);
    s
}

/// A STEP real or integer; `None` for anything else.
fn parse_real(a: &str) -> Option<f64> {
    let first = a.chars().next()?;
    if !(first.is_ascii_digit() || first == '-' || first == '+' || first == '.') {
        return None;
    }
    // `1.E-05`: a mantissa ending in its point.
    a.replace(".E", ".0E").replace(".e", ".0e").parse().ok()
}

/// A STEP real: always a decimal point, never an exponent, at most nine
/// decimals (a nanometre in metres), trailing zeros dropped.
fn format_real(v: f64) -> String {
    let s = format!("{v:.9}");
    let s = s.trim_end_matches('0');
    let s = if s == "-0." { "0." } else { s };
    s.to_string()
}

/// The attributes of `args`.
fn parse_args(args: &str) -> Result<Vec<Arg>, String> {
    let bytes = args.as_bytes();
    let mut pos = 0;
    let out = parse_list_body(bytes, &mut pos)?;
    if pos != bytes.len() {
        return Err(format!("unexpected text in STEP attributes: {args}"));
    }
    Ok(out)
}

fn parse_list_body(b: &[u8], pos: &mut usize) -> Result<Vec<Arg>, String> {
    let mut items = Vec::new();
    loop {
        skip_ws(b, pos);
        if *pos >= b.len() || b[*pos] == b')' {
            return Ok(items);
        }
        items.push(parse_arg(b, pos)?);
        skip_ws(b, pos);
        if *pos < b.len() && b[*pos] == b',' {
            *pos += 1;
        } else {
            return Ok(items);
        }
    }
}

fn skip_ws(b: &[u8], pos: &mut usize) {
    while *pos < b.len() && b[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
}

fn parse_arg(b: &[u8], pos: &mut usize) -> Result<Arg, String> {
    let start = *pos;
    match b[*pos] {
        b'\'' => {
            *pos += 1;
            loop {
                if *pos >= b.len() {
                    return Err("unterminated STEP string".into());
                }
                if b[*pos] == b'\'' {
                    if b.get(*pos + 1) == Some(&b'\'') {
                        *pos += 2;
                        continue;
                    }
                    *pos += 1;
                    break;
                }
                *pos += 1;
            }
            Ok(Arg::Atom(text_of(b, start, *pos)))
        }
        b'#' => {
            *pos += 1;
            while *pos < b.len() && b[*pos].is_ascii_digit() {
                *pos += 1;
            }
            text_of(b, start + 1, *pos)
                .parse()
                .map(Arg::Ref)
                .map_err(|_| "bad STEP reference".to_string())
        }
        b'(' => {
            *pos += 1;
            let items = parse_list_body(b, pos)?;
            expect_close(b, pos)?;
            Ok(Arg::List(items))
        }
        c if c.is_ascii_alphabetic() => {
            while *pos < b.len() && (b[*pos].is_ascii_alphanumeric() || b[*pos] == b'_') {
                *pos += 1;
            }
            let name = text_of(b, start, *pos);
            skip_ws(b, pos);
            if *pos < b.len() && b[*pos] == b'(' {
                *pos += 1;
                let items = parse_list_body(b, pos)?;
                expect_close(b, pos)?;
                Ok(Arg::Typed(name, items))
            } else {
                Ok(Arg::Atom(name))
            }
        }
        _ => {
            while *pos < b.len() && !matches!(b[*pos], b',' | b')') {
                *pos += 1;
            }
            Ok(Arg::Atom(text_of(b, start, *pos).trim().to_string()))
        }
    }
}

fn expect_close(b: &[u8], pos: &mut usize) -> Result<(), String> {
    skip_ws(b, pos);
    if *pos < b.len() && b[*pos] == b')' {
        *pos += 1;
        Ok(())
    } else {
        Err("unbalanced parentheses in STEP attributes".into())
    }
}

fn text_of(b: &[u8], from: usize, to: usize) -> String {
    String::from_utf8_lossy(&b[from..to]).into_owned()
}

/// The `#n` references in `args`, strings skipped.
fn refs_in(args: &str) -> Vec<u32> {
    let b = args.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut quoted = false;
    while i < b.len() {
        match b[i] {
            b'\'' => quoted = !quoted,
            b'#' if !quoted => {
                let start = i + 1;
                let mut j = start;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if let Ok(r) = args[start..j].parse() {
                    out.push(r);
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The IFC4 file as instances.
struct StepFile {
    header: String,
    footer: String,
    order: Vec<u32>,
    instances: HashMap<u32, Instance>,
    next_id: u32,
}

impl StepFile {
    fn parse(step: &str) -> Result<StepFile, String> {
        let data_at = step
            .find("\nDATA;")
            .ok_or("no DATA section in the IFC file")?;
        let body_start = data_at + "\nDATA;".len();
        let mut file = StepFile {
            header: step[..body_start].to_string(),
            footer: String::new(),
            order: Vec::new(),
            instances: HashMap::new(),
            next_id: 1,
        };
        let b = step.as_bytes();
        let mut i = body_start;
        loop {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= b.len() {
                return Err("no ENDSEC after the DATA section".into());
            }
            if b[i] != b'#' {
                file.footer = step[i..].to_string();
                break;
            }
            // `#id=NAME(args);`, strings skipped when looking for the `;`.
            let start = i;
            let mut quoted = false;
            while i < b.len() && (quoted || b[i] != b';') {
                if b[i] == b'\'' {
                    quoted = !quoted;
                }
                i += 1;
            }
            let text = &step[start..i];
            i += 1;
            let (id, rest) = text[1..]
                .split_once('=')
                .ok_or_else(|| format!("bad STEP instance: {text}"))?;
            let id: u32 = id
                .trim()
                .parse()
                .map_err(|_| format!("bad STEP instance id: {text}"))?;
            let rest = rest.trim();
            let open = rest
                .find('(')
                .ok_or_else(|| format!("bad STEP instance: {text}"))?;
            let close = rest
                .rfind(')')
                .ok_or_else(|| format!("bad STEP instance: {text}"))?;
            file.instances.insert(
                id,
                Instance {
                    name: rest[..open].trim().to_ascii_uppercase(),
                    args: rest[open + 1..close].to_string(),
                },
            );
            file.order.push(id);
            file.next_id = file.next_id.max(id + 1);
        }
        Ok(file)
    }

    fn write(&self) -> String {
        let mut out = String::with_capacity(self.header.len() + self.instances.len() * 64);
        out.push_str(&self.header);
        out.push('\n');
        for id in &self.order {
            if let Some(inst) = self.instances.get(id) {
                let _ = writeln!(out, "#{id}={}({});", inst.name, inst.args);
            }
        }
        out.push_str(&self.footer);
        out
    }

    fn add(&mut self, name: &str, args: String) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.instances.insert(
            id,
            Instance {
                name: name.into(),
                args,
            },
        );
        self.order.push(id);
        id
    }

    fn args(&self, id: u32) -> Option<Vec<Arg>> {
        parse_args(&self.instances.get(&id)?.args).ok()
    }

    fn name(&self, id: u32) -> Option<&str> {
        self.instances.get(&id).map(|i| i.name.as_str())
    }

    fn set_args(&mut self, id: u32, args: &[Arg]) {
        if let Some(inst) = self.instances.get_mut(&id) {
            inst.args = args_string(args);
        }
    }

    fn ids_named(&self, name: &str) -> Vec<u32> {
        self.order
            .iter()
            .copied()
            .filter(|id| self.name(*id) == Some(name))
            .collect()
    }

    // ---- profile and categories ------------------------------------------

    fn leave_out(
        &mut self,
        options: &IfcOptions,
        report: &mut IfcOptionsReport,
    ) -> Result<(), String> {
        if options.profile == IfcProfile::Full && options.categories.is_empty() {
            return Ok(());
        }
        let mut inbound: HashSet<u32> = HashSet::new();
        for inst in self.instances.values() {
            inbound.extend(refs_in(&inst.args));
        }
        let mut deleted: BTreeSet<u32> = BTreeSet::new();

        // Property sets the profile leaves out.
        if options.profile != IfcProfile::Full {
            for id in self.order.clone() {
                let name = self.name(id).unwrap_or_default();
                let leave = match name {
                    "IFCELEMENTQUANTITY" => true,
                    "IFCPROPERTYSET" => {
                        let set = self
                            .args(id)
                            .and_then(|a| a.get(2).and_then(Arg::text))
                            .unwrap_or_default();
                        set != super::revit_identity::SET
                            && (options.profile == IfcProfile::Geometry
                                || !set.starts_with("Pset_"))
                    }
                    _ => false,
                };
                if leave {
                    deleted.insert(id);
                    report.property_sets_left_out += 1;
                }
            }
        }

        // Relations, read before anything is deleted.
        let mut parent_of: HashMap<u32, u32> = HashMap::new();
        let mut dependants: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut set_category: HashMap<u32, String> = HashMap::new();
        let mut object_sets: Vec<(Vec<u32>, u32)> = Vec::new();
        let mut candidates: BTreeSet<u32> = BTreeSet::new();
        let mut contained_in: HashMap<u32, u32> = HashMap::new();
        if !options.categories.is_empty() {
            for id in self.order.clone() {
                let name = self.name(id).unwrap_or_default().to_string();
                let Some(args) = self.args(id) else { continue };
                match name.as_str() {
                    "IFCRELAGGREGATES" | "IFCRELNESTS" | "IFCRELVOIDSELEMENT" => {
                        let (Some(whole), Some(parts)) = (
                            args.get(4).and_then(Arg::target),
                            args.get(5).map(Arg::refs),
                        ) else {
                            continue;
                        };
                        for &p in &parts {
                            parent_of.entry(p).or_insert(whole);
                        }
                        dependants
                            .entry(whole)
                            .or_default()
                            .extend(parts.iter().copied());
                        if name == "IFCRELAGGREGATES" {
                            candidates.extend(parts);
                        }
                    }
                    "IFCRELCONTAINEDINSPATIALSTRUCTURE" => {
                        let elements = args.get(4).map(Arg::refs).unwrap_or_default();
                        for &e in &elements {
                            contained_in.entry(e).or_insert(id);
                        }
                        candidates.extend(elements);
                    }
                    "IFCRELDEFINESBYTYPE" => {
                        candidates.extend(args.get(4).map(Arg::refs).unwrap_or_default());
                    }
                    "IFCRELDEFINESBYPROPERTIES" => {
                        if let (Some(objects), Some(set)) = (
                            args.get(4).map(Arg::refs),
                            args.get(5).and_then(Arg::target),
                        ) {
                            object_sets.push((objects, set));
                        }
                    }
                    "IFCPROPERTYSET" => {
                        let set = args.get(2).and_then(Arg::text).unwrap_or_default();
                        if set == super::revit_identity::SET
                            || set == super::export_content::REVIT_OTHER_SET
                        {
                            for p in args.get(4).map(Arg::refs).unwrap_or_default() {
                                let Some(pa) = self.args(p) else { continue };
                                if pa.first().and_then(Arg::text).as_deref() == Some("Category")
                                    && let Some(c) = pa.get(2).and_then(Arg::text)
                                {
                                    set_category.insert(id, c);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            let mut category: HashMap<u32, String> = HashMap::new();
            for (objects, set) in &object_sets {
                if let Some(c) = set_category.get(set) {
                    for o in objects {
                        category.entry(*o).or_insert_with(|| c.clone());
                    }
                }
            }
            // Types carry their sets themselves.
            let mut types: Vec<u32> = Vec::new();
            for id in &self.order {
                let name = self.name(*id).unwrap_or_default();
                if !name.ends_with("TYPE") || name.starts_with("IFCREL") {
                    continue;
                }
                let Some(args) = self.args(*id) else { continue };
                if args.first().and_then(Arg::text).is_none() {
                    continue;
                }
                types.push(*id);
                for set in args.get(5).map(Arg::refs).unwrap_or_default() {
                    if let Some(c) = set_category.get(&set) {
                        category.entry(*id).or_insert_with(|| c.clone());
                    }
                }
            }
            candidates.retain(|id| {
                !matches!(
                    self.name(*id),
                    Some(
                        "IFCPROJECT"
                            | "IFCSITE"
                            | "IFCBUILDING"
                            | "IFCBUILDINGSTOREY"
                            | "IFCOPENINGELEMENT"
                            | "IFCDISTRIBUTIONPORT"
                    )
                )
            });
            let wanted: Vec<String> = options
                .categories
                .iter()
                .map(|c| c.trim().to_ascii_lowercase())
                .filter(|c| !c.is_empty())
                .collect();
            let mut matched: BTreeSet<&str> = BTreeSet::new();
            let mut keeps = |class: &str, category: Option<&String>| -> bool {
                let class = class.to_ascii_lowercase();
                let category = category.map(|c| c.to_ascii_lowercase());
                let mut hit = false;
                for w in &wanted {
                    let class_hit = w.starts_with("ifc")
                        && (class == *w || class.strip_suffix("type") == Some(w.as_str()));
                    if class_hit || category.as_deref() == Some(w.as_str()) {
                        matched.insert(w.as_str());
                        hit = true;
                    }
                }
                hit
            };
            let mut left_out_elements: BTreeSet<u32> = BTreeSet::new();
            let mut kept: BTreeSet<u32> = BTreeSet::new();
            for &id in &candidates {
                let class = self.name(id).unwrap_or_default().to_string();
                if keeps(&class, category.get(&id)) {
                    report.elements_kept += 1;
                    kept.insert(id);
                } else {
                    left_out_elements.insert(id);
                }
            }
            // A type no relation names (a type no element is placed of) goes
            // by its own category; any other type goes when its elements do.
            for &id in &types {
                if inbound.contains(&id) {
                    continue;
                }
                let class = self.name(id).unwrap_or_default().to_string();
                if !keeps(&class, category.get(&id)) {
                    deleted.insert(id);
                }
            }
            report.unmatched_categories = options
                .categories
                .iter()
                .filter(|c| !matched.contains(c.trim().to_ascii_lowercase().as_str()))
                .cloned()
                .collect();
            // Openings, ports and parts go with their element, unless a part
            // is kept itself.
            let mut stack: Vec<u32> = left_out_elements.iter().copied().collect();
            while let Some(id) = stack.pop() {
                if kept.contains(&id) || !deleted.insert(id) {
                    continue;
                }
                report.elements_left_out += 1;
                if let Some(parts) = dependants.get(&id) {
                    stack.extend(parts.iter().copied());
                }
            }
            // A kept part whose whole is left out joins the whole's storey
            // (the containment of its nearest contained ancestor).
            let mut joins: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for &id in &kept {
                if contained_in.contains_key(&id) {
                    continue;
                }
                let mut up = parent_of.get(&id).copied();
                if !up.is_some_and(|p| deleted.contains(&p)) {
                    continue;
                }
                while let Some(p) = up {
                    if let Some(&rel) = contained_in.get(&p) {
                        joins.entry(rel).or_default().push(id);
                        break;
                    }
                    up = parent_of.get(&p).copied();
                }
            }
            for (rel, parts) in joins {
                let Some(mut args) = self.args(rel) else {
                    continue;
                };
                if let Some(Arg::List(items)) = args.get_mut(4) {
                    items.extend(parts.into_iter().map(Arg::Ref));
                    self.set_args(rel, &args);
                }
            }
        }

        // Relations that named what was left out lose it, or go.
        let mut changed = true;
        while changed {
            changed = false;
            for id in self.order.clone() {
                if deleted.contains(&id) {
                    continue;
                }
                let refs = refs_in(&self.instances[&id].args);
                if !refs.iter().any(|r| deleted.contains(r)) {
                    continue;
                }
                let name = self.instances[&id].name.clone();
                let Some(mut args) = self.args(id) else {
                    return Err(format!("cannot read #{id}={name}"));
                };
                let is_relation = name.starts_with("IFCREL");
                let mut gone = false;
                for (k, a) in args.iter_mut().enumerate() {
                    match a {
                        Arg::Ref(r) if deleted.contains(r) => {
                            // An element in a room left out moves to the room's storey.
                            if name == "IFCRELCONTAINEDINSPATIALSTRUCTURE" && k == 5 {
                                let mut up = parent_of.get(r).copied();
                                while let Some(p) = up.filter(|p| deleted.contains(p)) {
                                    up = parent_of.get(&p).copied();
                                }
                                if let Some(p) = up {
                                    *a = Arg::Ref(p);
                                    continue;
                                }
                            }
                            gone = true;
                        }
                        Arg::List(items) => {
                            let before = items.len();
                            items.retain(|i| !i.target().is_some_and(|r| deleted.contains(&r)));
                            if items.is_empty() && before > 0 {
                                if is_relation {
                                    gone = true;
                                } else {
                                    *a = Arg::Atom("$".into());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                // A reference left deeper than the lists above: the instance goes.
                if !gone
                    && refs_in(&args_string(&args))
                        .iter()
                        .any(|r| deleted.contains(r))
                {
                    gone = true;
                }
                if gone {
                    deleted.insert(id);
                } else {
                    self.set_args(id, &args);
                }
                changed = true;
            }
        }
        for id in &deleted {
            self.instances.remove(id);
        }

        // Keep what the project, the relations and the free-standing roots
        // still reach; a styled item or colour map only with its item.
        let mut reached: HashSet<u32> = HashSet::new();
        let mut stack: Vec<u32> = Vec::new();
        let mut weak: Vec<u32> = Vec::new();
        for id in &self.order {
            let Some(inst) = self.instances.get(id) else {
                continue;
            };
            let root = inst.name == "IFCPROJECT" || inst.name.starts_with("IFCREL");
            let free = !inbound.contains(id);
            if root || (free && inst.args.starts_with('\'')) {
                stack.push(*id);
            } else if free {
                weak.push(*id);
            }
        }
        loop {
            while let Some(id) = stack.pop() {
                if !reached.insert(id) {
                    continue;
                }
                if let Some(inst) = self.instances.get(&id) {
                    stack.extend(refs_in(&inst.args));
                }
            }
            let before = weak.len();
            weak.retain(|id| {
                let first = self
                    .instances
                    .get(id)
                    .and_then(|i| refs_in(&i.args).first().copied());
                if first.is_some_and(|f| reached.contains(&f)) {
                    stack.push(*id);
                    false
                } else {
                    true
                }
            });
            if weak.len() == before {
                break;
            }
        }
        let unreached: Vec<u32> = self
            .instances
            .keys()
            .copied()
            .filter(|id| !reached.contains(id))
            .collect();
        report.instances_dropped = unreached.len();
        for id in unreached {
            self.instances.remove(&id);
        }
        self.order.retain(|id| self.instances.contains_key(id));
        Ok(())
    }

    // ---- origin ----------------------------------------------------------

    fn move_origin(&mut self, origin: IfcOrigin, g: &Georeference) -> Result<(), String> {
        // F(p) = R (p - o): R the turn to true north (survey point) or none.
        let (o, turn) = match origin {
            IfcOrigin::Internal => return Ok(()),
            IfcOrigin::ProjectBase => (
                g.project_base_point
                    .ok_or("--ifc-origin project-base: the project base point was not read")?
                    .internal,
                false,
            ),
            IfcOrigin::SurveyPoint => (
                g.survey_point
                    .ok_or("--ifc-origin survey-point: the survey point was not read")?
                    .internal,
                true,
            ),
        };
        let (s, c) = if turn {
            g.true_north.to_radians().sin_cos()
        } else {
            (0.0, 1.0)
        };
        let r = |p: [f64; 3]| [c * p[0] + s * p[1], -s * p[0] + c * p[1], p[2]];

        let site = *self
            .ids_named("IFCSITE")
            .first()
            .ok_or("no IfcSite in the IFC file")?;
        let site_args = self.args(site).ok_or("cannot read the IfcSite")?;
        let placement = site_args
            .get(5)
            .and_then(Arg::target)
            .ok_or("the IfcSite has no placement")?;
        let mut placement_args = self
            .args(placement)
            .ok_or("cannot read the site placement")?;
        let axis = placement_args.get(1).and_then(Arg::target);
        // The site's own placement (identity as written), composed with F.
        let (mut loc, mut z, mut x) = ([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        if let Some(axis) = axis {
            let a = self
                .args(axis)
                .ok_or("cannot read the site axis placement")?;
            if let Some(p) = a.first().and_then(Arg::target) {
                loc = self.triple(p).unwrap_or(loc);
            }
            if let Some(d) = a.get(1).and_then(Arg::target) {
                z = self.triple(d).unwrap_or(z);
            }
            if let Some(d) = a.get(2).and_then(Arg::target) {
                x = self.triple(d).unwrap_or(x);
            }
        }
        let loc = r([loc[0] - o[0], loc[1] - o[1], loc[2] - o[2]]);
        let (z, x) = (r(z), r(x));
        let point = self.add("IFCCARTESIANPOINT", format!("({})", reals(&loc)));
        let zd = self.add("IFCDIRECTION", format!("({})", reals(&z)));
        let xd = self.add("IFCDIRECTION", format!("({})", reals(&x)));
        let new_axis = self.add("IFCAXIS2PLACEMENT3D", format!("#{point},#{zd},#{xd}"));
        if placement_args.len() < 2 {
            return Err("unexpected site placement".into());
        }
        placement_args[1] = Arg::Ref(new_axis);
        self.set_args(placement, &placement_args);

        // The map coordinates of every point stay: the new origin's shared
        // coordinates, and the shared axes (survey point) or the turn of the
        // internal ones (project base point).
        if let Some(crs) = &g.crs {
            for id in self.ids_named("IFCMAPCONVERSION") {
                let Some(mut a) = self.args(id) else { continue };
                if a.len() < 7 {
                    continue;
                }
                let shared = g.to_shared(o);
                for k in 0..3 {
                    a[2 + k] = Arg::Atom(format_real(crs.map_value(shared[k])));
                }
                if turn {
                    a[5] = Arg::Atom(format_real(1.0));
                    a[6] = Arg::Atom(format_real(0.0));
                }
                self.set_args(id, &a);
            }
        }
        Ok(())
    }

    /// The three numbers of an `IfcCartesianPoint` or `IfcDirection`.
    fn triple(&self, id: u32) -> Option<[f64; 3]> {
        let a = self.args(id)?;
        let Arg::List(items) = a.first()? else {
            return None;
        };
        let mut v = [0.0; 3];
        for (k, item) in items.iter().take(3).enumerate() {
            let Arg::Atom(t) = item else { return None };
            v[k] = parse_real(t)?;
        }
        Some(v)
    }

    // ---- units -----------------------------------------------------------

    fn rescale(&mut self, unit: IfcLengthUnit) -> Result<(), String> {
        let kl = unit.per_metre();
        let (ka, kv) = unit.area_volume_factor();
        let first_new = self.next_id;
        // The project's units: replace its length (and, for imperial, area
        // and volume) unit in the IfcUnitAssignment.
        let assignment = *self
            .ids_named("IFCUNITASSIGNMENT")
            .first()
            .ok_or("no IfcUnitAssignment in the IFC file")?;
        let mut assignment_args = self.args(assignment).ok_or("cannot read the units")?;
        let Some(Arg::List(units)) = assignment_args.first_mut() else {
            return Err("unexpected IfcUnitAssignment".into());
        };
        let mut metre: Option<u32> = None;
        for unit_ref in units.iter_mut() {
            let Some(id) = unit_ref.target() else {
                continue;
            };
            let Some(args) = self.args(id) else { continue };
            let kind = args.get(1).map(|a| {
                let mut s = String::new();
                a.write(&mut s);
                s
            });
            let replacement = match (kind.as_deref(), unit) {
                (Some(".LENGTHUNIT."), IfcLengthUnit::Millimetre) => {
                    Some(self.add("IFCSIUNIT", "*,.LENGTHUNIT.,.MILLI.,.METRE.".into()))
                }
                (Some(".LENGTHUNIT."), IfcLengthUnit::Foot | IfcLengthUnit::Inch) => {
                    Some(self.conversion_unit(&mut metre, unit, 1))
                }
                (Some(".AREAUNIT."), IfcLengthUnit::Foot | IfcLengthUnit::Inch) => {
                    Some(self.conversion_unit(&mut metre, unit, 2))
                }
                (Some(".VOLUMEUNIT."), IfcLengthUnit::Foot | IfcLengthUnit::Inch) => {
                    Some(self.conversion_unit(&mut metre, unit, 3))
                }
                _ => None,
            };
            if let Some(r) = replacement {
                *unit_ref = Arg::Ref(r);
            }
        }
        self.set_args(assignment, &assignment_args);
        for id in self.order.clone() {
            if id >= first_new {
                continue;
            }
            let name = self.instances[&id].name.clone();
            let lengths: &[usize] = match name.as_str() {
                "IFCCARTESIANPOINT" | "IFCCARTESIANPOINTLIST2D" | "IFCCARTESIANPOINTLIST3D" => &[0],
                "IFCEXTRUDEDAREASOLID" => &[3],
                "IFCRECTANGLEPROFILEDEF" => &[3, 4],
                "IFCRECTANGLEHOLLOWPROFILEDEF" => &[3, 4, 5, 6, 7],
                "IFCCIRCLEPROFILEDEF" => &[3],
                "IFCCIRCLEHOLLOWPROFILEDEF" => &[3, 4],
                "IFCISHAPEPROFILEDEF" | "IFCUSHAPEPROFILEDEF" => &[3, 4, 5, 6, 7, 8],
                "IFCLSHAPEPROFILEDEF" => &[3, 4, 5, 6, 7],
                "IFCTSHAPEPROFILEDEF" => &[3, 4, 5, 6, 7, 8, 9],
                "IFCCIRCLE" => &[1],
                "IFCBUILDINGSTOREY" => &[9],
                "IFCBUILDING" => &[9, 10],
                "IFCSITE" => &[11],
                "IFCSPACE" => &[10],
                "IFCDOOR" | "IFCWINDOW" => &[8, 9],
                "IFCSTAIRFLIGHT" => &[10, 11],
                "IFCMATERIALLAYER" => &[1],
                "IFCMATERIALLAYERSETUSAGE" => &[3, 4],
                "IFCMATERIALPROFILESETUSAGE" => &[2],
                "IFCGEOMETRICREPRESENTATIONCONTEXT" => &[3],
                _ => &[],
            };
            let quantity = match name.as_str() {
                "IFCQUANTITYLENGTH" => Some(kl),
                "IFCQUANTITYAREA" => Some(ka),
                "IFCQUANTITYVOLUME" => Some(kv),
                _ => None,
            };
            let typed_ok = name != "IFCMEASUREWITHUNIT";
            let inst_args = &self.instances[&id].args;
            let has_typed = typed_ok
                && (inst_args.contains("LENGTHMEASURE(")
                    || inst_args.contains("IFCAREAMEASURE(")
                    || inst_args.contains("IFCVOLUMEMEASURE("));
            if lengths.is_empty() && quantity.is_none() && !has_typed && name != "IFCMAPCONVERSION"
            {
                continue;
            }
            let mut args = self
                .args(id)
                .ok_or_else(|| format!("cannot read #{id}={name}"))?;
            for &k in lengths {
                if let Some(a) = args.get_mut(k) {
                    a.scale(kl);
                }
            }
            // A value with its own unit keeps it.
            let own_unit =
                |args: &[Arg], k: usize| args.get(k).is_some_and(|a| *a != Arg::Atom("$".into()));
            if let Some(q) = quantity
                && !own_unit(&args, 2)
                && let Some(a) = args.get_mut(3)
            {
                a.scale(q);
            }
            if name == "IFCMAPCONVERSION" {
                // Scale takes the project's length unit to the map's.
                if let Some(a) = args.get_mut(7) {
                    let old = match a {
                        Arg::Atom(t) => parse_real(t).unwrap_or(1.0),
                        _ => 1.0,
                    };
                    *a = Arg::Atom(format_real(old / kl));
                }
            }
            let property_unit = matches!(
                name.as_str(),
                "IFCPROPERTYSINGLEVALUE" | "IFCPROPERTYLISTVALUE"
            ) && own_unit(&args, 3);
            if has_typed && !property_unit {
                for a in args.iter_mut() {
                    scale_typed(a, kl, ka, kv);
                }
            }
            self.set_args(id, &args);
        }
        Ok(())
    }

    /// An `IfcConversionBasedUnit` of feet or inches to the power `power`.
    fn conversion_unit(&mut self, metre: &mut Option<u32>, unit: IfcLengthUnit, power: i32) -> u32 {
        let base = match power {
            1 => "*,.LENGTHUNIT.,$,.METRE.",
            2 => "*,.AREAUNIT.,$,.SQUARE_METRE.",
            _ => "*,.VOLUMEUNIT.,$,.CUBIC_METRE.",
        };
        let si = if power == 1 {
            *metre.get_or_insert_with(|| self.add("IFCSIUNIT", base.into()))
        } else {
            self.add("IFCSIUNIT", base.into())
        };
        let dims = self.add("IFCDIMENSIONALEXPONENTS", format!("{power},0,0,0,0,0,0"));
        let metres = (1.0 / unit.per_metre()).powi(power);
        let (measure, kind, name) = match power {
            1 => ("IFCLENGTHMEASURE", ".LENGTHUNIT.", ""),
            2 => ("IFCAREAMEASURE", ".AREAUNIT.", "SQUARE "),
            _ => ("IFCVOLUMEMEASURE", ".VOLUMEUNIT.", "CUBIC "),
        };
        let word = if unit == IfcLengthUnit::Foot {
            "FOOT"
        } else {
            "INCH"
        };
        let m = self.add(
            "IFCMEASUREWITHUNIT",
            format!("{measure}({}),#{si}", format_real_precise(metres)),
        );
        self.add(
            "IFCCONVERSIONBASEDUNIT",
            format!("#{dims},{kind},'{name}{word}',#{m}"),
        )
    }
}

/// Scale the length, area and volume measures in `a`.
fn scale_typed(a: &mut Arg, kl: f64, ka: f64, kv: f64) {
    match a {
        Arg::Typed(name, items) => {
            let k = match name.as_str() {
                "IFCLENGTHMEASURE" | "IFCPOSITIVELENGTHMEASURE" | "IFCNONNEGATIVELENGTHMEASURE" => {
                    Some(kl)
                }
                "IFCAREAMEASURE" => Some(ka),
                "IFCVOLUMEMEASURE" => Some(kv),
                _ => None,
            };
            match k {
                Some(k) => items.iter_mut().for_each(|i| i.scale(k)),
                None => items.iter_mut().for_each(|i| scale_typed(i, kl, ka, kv)),
            }
        }
        Arg::List(items) => items.iter_mut().for_each(|i| scale_typed(i, kl, ka, kv)),
        _ => {}
    }
}

/// A conversion factor written in full (`0.09290304`).
fn format_real_precise(v: f64) -> String {
    let s = format!("{v:?}");
    if s.contains('e') {
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = if m.contains('.') {
            m.to_string()
        } else {
            format!("{m}.")
        };
        format!("{m}E{e}")
    } else if s.contains('.') {
        s
    } else {
        format!("{s}.")
    }
}

fn reals(v: &[f64; 3]) -> String {
    v.iter()
        .map(|x| format_real(*x))
        .collect::<Vec<_>>()
        .join(",")
}
