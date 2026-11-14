//! `rvt-ifc` — convert a Revit file to IFC4.
//!
//! Writes the project, its storeys and the elements rvt-open decodes as typed
//! IFC4 entities, each with its body where the geometry is recovered and its
//! bounding box where it is not (`docs/support-matrix.json` rates each
//! category). The same input gives the same bytes, apart from the time in
//! the header and the owner history, which `SOURCE_DATE_EPOCH` fixes.

use clap::{Parser, ValueEnum};
use rvt::{
    RevitFile,
    ifc::{
        ExportConvention, ExportDiagnostics, ExportQualityMode, write_ifcx_graph_layers_to,
        write_usda_graph, write_usdc_graph,
    },
    walker::WalkerLimits,
};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum CliExportQualityMode {
    Scaffold,
    TypedNoGeometry,
    Geometry,
    Strict,
}

impl From<CliExportQualityMode> for ExportQualityMode {
    fn from(value: CliExportQualityMode) -> Self {
        match value {
            CliExportQualityMode::Scaffold => Self::Scaffold,
            CliExportQualityMode::TypedNoGeometry => Self::TypedNoGeometry,
            CliExportQualityMode::Geometry => Self::Geometry,
            CliExportQualityMode::Strict => Self::Strict,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum CliConvention {
    Revit,
    Open,
}

impl From<CliConvention> for ExportConvention {
    fn from(value: CliConvention) -> Self {
        match value {
            CliConvention::Revit => Self::Revit,
            CliConvention::Open => Self::Open,
        }
    }
}

#[derive(Parser)]
#[command(
    version,
    about = "Convert a Revit file to IFC4",
    after_help = "Examples:\n  \
        rvt-ifc model.rvt                     writes model.ifc next to the input\n  \
        rvt-ifc model.rvt -o out.ifc --diagnostics out.diagnostics.json\n  \
        rvt-ifc model.rvt --diagnostics out.diagnostics.json --anonymize   to post publicly\n  \
        rvt-ifc model.rvt --diagnostics out.diagnostics.json --element-provenance   list where each element was read from\n  \
        rvt-ifc model.rvt --diagnostics out.diagnostics.json --export-decisions   list why each element was left out\n  \
        rvt-ifc model.rvt --mode geometry\n  \
        rvt-ifc model.rvt --ifc-profile coordination --ifc-units mm --ifc-origin survey-point\n  \
        rvt-ifc model.rvt --ifc-categories Walls,Doors,IfcSlab --phase \"Existing\" --report run.json\n\n\
        SOURCE_DATE_EPOCH (Unix seconds) sets the time the IFC header and owner\n\
        history record, so two exports of the same file are the same bytes.\n\n\
        Exit status 3: a resource limit kept part of the file from being read, so\n\
        no IFC was written (the diagnostics sidecar, if asked for, lists the\n\
        streams). Re-run with --large-model, or --allow-partial to write it anyway.\n\
        A reader that stopped at its work budget is a warning: the IFC is written."
)]
struct Args {
    /// Path to a `.rvt` / `.rfa` / `.rte` / `.rft` file.
    input: PathBuf,
    /// Output path. If omitted, writes `<input>.ifc` next to the input.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Use the placeholder exporter (empty project body) instead of the
    /// document exporter. Mostly useful for testing the STEP writer.
    /// Kept as `--null` for backward compatibility with earlier versions.
    #[arg(long, alias = "null", conflicts_with = "diagnostic_proxies")]
    placeholder: bool,
    /// Require a minimum export quality before writing IFC.
    ///
    /// `scaffold` preserves historical behavior and accepts a valid IFC4
    /// framework even when real elements are missing. `typed-no-geometry`
    /// requires validated typed IFC elements. `geometry` requires at least
    /// one exported element with geometry. `strict` also requires recovered
    /// project metadata, units, levels, and zero export warnings.
    #[arg(long, value_enum, default_value = "scaffold")]
    mode: CliExportQualityMode,
    /// writes (classes, containment, property sets); `open` writes what the
    /// model means, where Revit's export bends it.
    #[arg(
        long,
        value_enum,
        default_value = "revit",
        conflicts_with = "placeholder"
    )]
    convention: CliConvention,
    /// Include low-confidence schema-scan candidates as
    /// IFCBUILDINGELEMENTPROXY entities with diagnostic provenance.
    ///
    /// Default export suppresses these candidates because they are not
    /// validated typed model elements.
    #[arg(long, conflicts_with = "placeholder")]
    diagnostic_proxies: bool,
    /// Draw every element whose body would be its record's bounding box
    /// (furniture, fixtures, equipment and the rest) from the mesh Revit
    /// saved for display, on Revit 2014 to 2027 (on by default; this flag is
    /// kept for old command lines). The walls that embed a curtain wall are
    /// drawn so on Revit 2024 to 2027 where Revit raised their base. Slower on
    /// large files.
    #[arg(long, conflicts_with_all = ["placeholder", "no_saved_meshes"])]
    saved_meshes: bool,
    /// draw those elements from the family's own embedded
    /// document (the loaded family's solid forms placed by the instance's
    /// transform) where the saved graphics leave a box; a body is used only
    /// when it is the size of the instance's record box.
    /// on by default (this flag is kept for old command lines).
    #[arg(long, conflicts_with_all = ["placeholder", "no_family_geometry"])]
    family_geometry: bool,
    /// Keep bounding-box bodies the saved graphics leave, instead of the
    /// family document's solids.
    #[arg(long, conflicts_with = "placeholder")]
    no_family_geometry: bool,
    /// with `--family-geometry`, try the family document before
    /// the saved graphics (default: the saved graphics first).
    #[arg(long, conflicts_with = "placeholder")]
    family_first: bool,
    /// give every element that has no material the one of its
    /// saved graphics' largest face group.
    /// on by default (this flag is kept for old command lines).
    #[arg(long, conflicts_with_all = ["placeholder", "no_saved_materials"])]
    saved_materials: bool,
    /// Leave elements without a material as they are.
    #[arg(long, conflicts_with = "placeholder")]
    no_saved_materials: bool,
    /// Keep trees, people and cars that no real shape decodes as their record's
    /// bounding box instead of an approximate proxy shape.
    #[arg(long, conflicts_with = "placeholder")]
    no_proxy_shapes: bool,
    /// Draw a family instance from its family's own extrusions (the sketch
    /// lines they were drawn from, swept between their offsets) instead of
    /// its saved mesh, where they reproduce the mesh; every other instance
    /// keeps it. An arc in a profile is written as the arc, whatever
    /// its radius. Off by default. Each instance is checked against its own
    /// mesh. Revit 2024 to 2027 files (measured on 2024 and 2025). Slower on
    /// large files.
    #[arg(long, conflicts_with_all = ["placeholder", "no_saved_meshes"])]
    family_extrusions: bool,
    /// Keep every element as its record's bounding box instead of the
    /// mesh Revit saved for it.
    #[arg(long, conflicts_with = "placeholder")]
    no_saved_meshes: bool,
    /// Revit's own property sets, as Revit's exporter does with "Export
    /// Revit property sets": each element's parameters (built-in ones
    /// named) in one set per parameter group, its type's on its type
    /// object, and a `Revit` set with its ElementId and category.
    /// existing scripts. Revit 2015 to 2027 files.
    #[arg(long, conflicts_with_all = ["placeholder", "no_revit_property_sets"])]
    revit_property_sets: bool,
    /// Leave out Revit's own property sets and the `Revit` set (faster on
    /// large files).
    #[arg(long)]
    no_revit_property_sets: bool,
    /// Also write properties where Revit's own exporter writes them and
    /// IFC4's definitions do not: a standard set whose entity's name begins
    /// with the element's (`Pset_StairFlightCommon` on a stair,
    /// `Pset_SpaceHeaterTypeCommon` on a space), its `Reference` only, and a
    /// text parameter named `<set>.<property>` for a property the set's IFC4
    /// definition lacks (`Pset_SpaceCommon.FloorCovering`). By
    /// default every property is in the set buildingSMART's IFC4 definitions
    /// put it in.
    #[arg(long, conflicts_with = "placeholder")]
    revit_set_placement: bool,
    /// Leave out the base quantities (the `Qto_` sets: lengths, areas and
    /// widths), as Revit's exporter does when its export setup's "Export
    /// base quantities" is off. Revit's built-in IFC4 setups turn it on, and
    /// so does rvt-ifc by default.
    #[arg(long)]
    no_base_quantities: bool,
    /// write the materials takeoff to this path, a CSV of each
    /// material's volume (m3), area (m2) and length (m) per Revit category,
    /// with the element count and whether it is measured from the drawn body
    /// or estimated from the bounding box an element is still drawn as.
    #[arg(long, value_name = "PATH")]
    takeoff: Option<PathBuf>,
    /// With `--takeoff`: leave the estimated (bounding-box) elements out.
    #[arg(long, requires = "takeoff")]
    takeoff_measured_only: bool,
    /// write each element's quantities to this path, a CSV of one
    /// row per element and material (volumes, areas, layer widths, lengths).
    #[arg(long, value_name = "PATH")]
    element_quantities: Option<PathBuf>,
    /// write every element's body as its own mesh (IFCX, USD and
    /// .frag: in world coordinates; IFC4: no IfcMappedItem), the form before
    /// shared shapes. Default: one shared shape per distinct element-local
    /// mesh, placed by the element (IFC4 IfcRepresentationMap + IfcMappedItem
    /// for solid-shape bodies; IFCX/USD/.frag a shared shape node per body).
    #[arg(long)]
    per_element_meshes: bool,
    /// Write a JSON diagnostics sidecar for bug reports and support.
    ///
    /// The sidecar includes input metadata, decoded/exported element counts,
    /// skipped low-confidence candidates, unsupported features, warnings, and
    /// an export confidence summary.
    #[arg(long, value_name = "PATH")]
    diagnostics: Option<PathBuf>,
    /// Replace every name the diagnostics sidecar takes from the project
    /// with a placeholder: the file and project names, levels, materials and
    /// sample element names. Counts and ElementIds stay, so the sidecar can
    /// be posted publicly.
    #[arg(long, requires = "diagnostics")]
    anonymize: bool,
    /// Also list, in the diagnostics sidecar, where each decoded element was
    /// read from: the kind of record, its stream and byte offset in
    /// the inflated stream (what rvt-dump writes as Partitions_NN.decomp),
    /// where its ElementId is stored, its class and its native record. Off by
    /// default: on a large model the list is hundreds of times the rest of
    /// the sidecar (docs/export-diagnostics.md, "Provenance table size", gives
    /// the measured sizes). Never written into the IFC.
    #[arg(long, requires = "diagnostics", conflicts_with = "placeholder")]
    element_provenance: bool,
    /// Also list, in the diagnostics sidecar, each fail-closed decision per
    /// element: why an element was left out, a body is only its
    /// record's box, an opening was not cut or an outline was not read, as
    /// its ElementId, a stage and a reason code, with the stream and byte
    /// offset its record was read from where known. The `skipped` counts are
    /// made from the same decisions. Bounded at 5,000 entries a reason, with
    /// the rest counted as dropped. Off by default. Never written into the IFC.
    #[arg(long, requires = "diagnostics", conflicts_with = "placeholder")]
    export_decisions: bool,
    /// Witness mode: write a witness observation of this input.
    ///
    /// The observation (docs/verification-protocol.md) records the
    /// SHA-256 of the input bytes, the exported entity counts by IFC type,
    /// the `IfcRelFillsElement` host/filling `Tag` pairs, the
    /// `IfcBuildingStorey` name/elevation set, and a canonical hash of
    /// that payload so an independent replay can prove this witness saw
    /// the same thing. Compared against other witnesses by
    /// tools/ci/witness-verdict.py.
    #[arg(long, value_name = "PATH")]
    observation: Option<PathBuf>,
    /// Golden-artifact id to stamp into the observation (registry id).
    #[arg(long, value_name = "ID", requires = "observation")]
    artifact_id: Option<String>,
    /// Also write the model as IFCX (IFC5 alpha) to this path: a flat mesh
    /// per element, metres, Revit's GlobalIds as node paths.
    #[arg(long, value_name = "PATH", conflicts_with = "placeholder")]
    ifcx: Option<PathBuf>,
    /// Write the IFCX loss ledger (what the IFC4 file holds that the IFCX
    /// does not) as JSON to this path.
    #[arg(long, value_name = "PATH", requires = "ifcx")]
    ifcx_ledger: Option<PathBuf>,
    /// Write the exporter's diagnostics (where in the RVT a record was
    /// found) as a separate IFCX layer to this path. The `--ifcx` file
    /// leaves them out, so it does not change when only the file layout
    /// does; composing it with this layer on top gives the full tree.
    #[arg(long, value_name = "PATH", requires = "ifcx")]
    ifcx_diagnostics: Option<PathBuf>,
    /// Leave out of the IFCX file what an IFC4 export does not hold: every
    /// other element the file saves parameters for (phases, design options,
    /// views, levels, materials, project information) and the
    /// parameters the exported elements lack (faster on large files).
    #[arg(long, requires = "ifcx")]
    no_ifcx_revit_elements: bool,
    /// Also write into the IFCX file the types no element uses (most come
    /// from the project template; a larger file).
    #[arg(long, requires = "ifcx", conflicts_with = "no_ifcx_revit_elements")]
    ifcx_unused_types: bool,
    /// Also write the model as ThatOpen Fragments (`.frag`, the public
    /// @thatopen/fragments 3.6.0 schema) to this path: the IFCX graph's
    /// items, properties, tree and meshes.
    #[arg(long, value_name = "PATH", conflicts_with = "placeholder")]
    frag: Option<PathBuf>,

    /// Give rooms and spaces their volumes in the `.frag`. Off
    /// (as Revit's 3D views), they keep their items and properties with no
    /// geometry.
    #[arg(long, requires = "frag")]
    frag_rooms: bool,
    /// Write the `.frag` counts (items, properties, triangles) as JSON.
    #[arg(long, value_name = "PATH", requires = "frag")]
    frag_stats: Option<PathBuf>,
    /// Also write the model as OpenUSD text (`.usda`) to this path: the same
    /// graph as IFCX, a UsdGeomMesh per element body, metres, Z up.
    #[arg(long, value_name = "PATH", conflicts_with = "placeholder")]
    usda: Option<PathBuf>,
    /// Also write the model as binary OpenUSD (`.usdc`, Crate 0.8.0) to this
    /// path: the same specs and values as `--usda`.
    #[arg(long, value_name = "PATH", conflicts_with = "placeholder")]
    usdc: Option<PathBuf>,
    /// Force the projected CRS to this EPSG code in every output (IFC4
    /// IfcProjectedCRS + IfcMapConversion, IFCX, USD, .frag), marked as
    /// user-supplied. The unit is metres unless the file's own CRS is in
    /// feet; no EPSG database is consulted.
    #[arg(long, value_name = "CODE", value_parser = clap::value_parser!(u32).range(1..))]
    epsg: Option<u32>,
    /// What the GlobalIds the writer generates (relations, property sets,
    /// and any entity Revit gives no GlobalId) are derived under. Default
    /// `rvt-open ifc GlobalId v1`. Revit's own GlobalIds are never affected.
    /// Also read from `RVT_OPEN_GUID_NAMESPACE`. Non-empty, at most 256
    /// bytes.
    #[arg(long, value_name = "TEXT", value_parser = parse_guid_namespace)]
    guid_namespace: Option<String>,
    /// Fixed IFC4 time stamp (RFC 3339, e.g. `2026-10-29T00:00:00Z`) for
    /// the `FILE_NAME` header and `IfcOwnerHistory.CreationDate`, so two
    /// exports of the same file are byte-identical. Without it the
    /// `SOURCE_DATE_EPOCH` environment variable (Unix seconds, the
    /// reproducible-builds convention) is used, else the current time.
    #[arg(long, value_name = "RFC3339", value_parser = parse_rfc3339)]
    ifc4_timestamp: Option<i64>,
    /// Maximum decompressed Global/Latest bytes scanned by the walker.
    #[arg(long)]
    max_walker_scan_bytes: Option<usize>,
    /// Maximum schema-scan candidates retained by the walker.
    #[arg(long)]
    max_walker_candidates: Option<usize>,
    /// Maximum trial decodes attempted by the walker.
    #[arg(long)]
    max_walker_trial_offsets: Option<usize>,
    /// Maximum bytes inspected while decoding one walker candidate.
    #[arg(long)]
    max_walker_record_decode_bytes: Option<usize>,
    /// Maximum records accepted in walker reference containers.
    #[arg(long)]
    max_walker_container_records: Option<usize>,
    /// Maximum bytes of one stream read from the file (default 256 MiB). A
    /// larger stream is not read, and nothing is written unless
    /// `--allow-partial`.
    #[arg(long, value_name = "BYTES")]
    max_stream_bytes: Option<u64>,
    /// Maximum inflated bytes of one compressed member (default 256 MiB). The
    /// members of one stream may inflate to this or 1 GiB together,
    /// whichever is larger.
    #[arg(long, value_name = "BYTES")]
    max_inflate_bytes: Option<usize>,
    /// Raise the stream, inflate and walker-scan limits for large models:
    /// streams up to 2 GiB, 8 GiB inflated. A model whose largest partition
    /// is 1 GiB stored needs several GB of memory.
    #[arg(long)]
    large_model: bool,
    /// Write the IFC even when a resource limit kept part of the file from
    /// being read. The diagnostics list the streams that were not read.
    #[arg(long)]
    allow_partial: bool,
    /// The IFC file's profile, units, origin, element filter, and the phase
    /// shown (docs/data-outputs.md).
    #[command(flatten)]
    ifc_output: rvt::cli::IfcOutputArgs,
    /// Write a run report (JSON) to this path: the files written, elements
    /// by category, the notes printed here, what the IFC options left out,
    /// and the export diagnostics (docs/data-outputs.md).
    #[arg(long, value_name = "PATH")]
    report: Option<PathBuf>,
}

/// `SOURCE_DATE_EPOCH` (reproducible-builds.org): unset or empty is `None`;
/// anything but non-negative integer seconds is an error.
fn source_date_epoch() -> anyhow::Result<Option<i64>> {
    match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(v) if !v.is_empty() => match v.parse::<i64>() {
            Ok(t) if t >= 0 => Ok(Some(t)),
            _ => anyhow::bail!("SOURCE_DATE_EPOCH {v:?} is not non-negative Unix seconds"),
        },
        _ => Ok(None),
    }
}

/// RFC 3339 date-time (`YYYY-MM-DDTHH:MM:SS[.frac](Z|+HH:MM|-HH:MM)`) as
/// Unix seconds, from 1970; fractions are dropped.
fn parse_rfc3339(s: &str) -> Result<i64, String> {
    let bad =
        || format!("{s:?} is not an RFC 3339 date-time from 1970 (e.g. 2026-10-29T00:00:00Z)");
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Result<i64, String> {
        let t = s.get(r).ok_or_else(bad)?;
        if t.bytes().all(|c| c.is_ascii_digit()) {
            t.parse().map_err(|_| bad())
        } else {
            Err(bad())
        }
    };
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return Err(bad());
    }
    let (y, mo, d, h, mi, sec) = (
        num(0..4)?,
        num(5..7)?,
        num(8..10)?,
        num(11..13)?,
        num(14..16)?,
        num(17..19)?,
    );
    let mut i = 19;
    if b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return Err(bad());
        }
    }
    let offset = match &s[i..] {
        "Z" | "z" => 0,
        o if o.len() == 6 && matches!(o.as_bytes()[0], b'+' | b'-') && o.as_bytes()[3] == b':' => {
            let m = num(i + 1..i + 3)? * 60 + num(i + 4..i + 6)?;
            if o.starts_with('-') { -m * 60 } else { m * 60 }
        }
        _ => return Err(bad()),
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&mo)
        || d < 1
        || d > dim[(mo - 1) as usize]
        || h > 23
        || mi > 59
        || sec > 60
    {
        return Err(bad());
    }
    // Days from civil (H. Hinnant's algorithm).
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    let t = days * 86_400 + h * 3600 + mi * 60 + sec.min(59) - offset;
    if t < 0 { Err(bad()) } else { Ok(t) }
}

/// A resource limit kept part of the input from being read, and
/// `--allow-partial` was not given: exit status 3.
#[derive(Debug)]
struct PartialRead;

impl std::fmt::Display for PartialRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no IFC written: part of the input was not read (see above)")
    }
}

impl std::error::Error for PartialRead {}

fn main() -> std::process::ExitCode {
    rvt::cli::exit_quietly_on_broken_pipe();
    match run(Args::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            if e.downcast_ref::<PartialRead>().is_some() {
                std::process::ExitCode::from(3)
            } else {
                std::process::ExitCode::from(1)
            }
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    rvt::cli::log_stage_memory();
    let started = std::time::Instant::now();
    let out_path = args.output.clone().unwrap_or_else(|| {
        let mut p = args.input.clone();
        p.set_extension("ifc");
        p
    });
    // Before the export, which can take minutes: the IFC would replace the file
    // it was read from.
    if rvt::cli::same_file(&out_path, &args.input) {
        anyhow::bail!(
            "refusing to overwrite the input {}; pass -o <path>",
            args.input.display()
        );
    }
    let mut rf = RevitFile::open_with_limits(
        &args.input,
        rvt::cli::open_limits(
            args.max_stream_bytes,
            args.max_inflate_bytes,
            args.large_model,
        ),
    )?;
    args.ifc_output.set_phase(&mut rf)?;
    let ifc_options = args.ifc_output.options();
    let mut opts = convert_options_from_args(&args);
    if opts.guid_namespace.is_none() {
        if let Ok(ns) = std::env::var("RVT_OPEN_GUID_NAMESPACE") {
            rvt::ifc::validate_guid_namespace(&ns)
                .map_err(|e| anyhow::anyhow!("RVT_OPEN_GUID_NAMESPACE: {e}"))?;
            opts.guid_namespace = Some(ns);
        }
    }
    let quality_mode = opts.quality;
    rvt::control::stage("export");

    // The pipeline is shared with rvt-wasm, so the browser's
    // output is byte-identical to this command's.
    let exported = rvt::convert::export(&mut rf, &opts)?;
    for note in &exported.notes {
        eprintln!("rvt-ifc: {note}");
    }
    let mut report =
        rvt::ifc::run_report::RunReport::new("rvt-ifc", &args.input.display().to_string());
    report.notes.clone_from(&exported.notes);
    if let Some(why) =
        rvt::ifc::ifc_options::origin_unavailable(&exported.model, ifc_options.origin)
    {
        anyhow::bail!("{why}; no IFC written");
    }
    if let Some(path) = &args.takeoff {
        let csv =
            rvt::ifc::quantities::takeoff_csv(&exported.quantities, !args.takeoff_measured_only);
        rvt::cli::write_output(path, csv)
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    }
    if let Some(path) = &args.element_quantities {
        let csv = rvt::ifc::quantities::element_quantities_csv(&exported.quantities);
        rvt::cli::write_output(path, csv)
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    }
    let mut model = exported.model;
    let mut diagnostics = exported.diagnostics;

    if args.revit_set_placement {
        // Like Revit's property sets, this only adds to the export: when the
        // parameters cannot be read, it goes ahead without them, and says so.
        match rvt::ifc::revit_set_placement::attach(&mut rf, &mut model) {
            Ok(report) => {
                rvt::ifc::recount_exported(&mut diagnostics, &model);
                eprintln!(
                    "rvt-ifc: {} properties on {} elements written where Revit's exporter writes them ({} parameters of a kind not written left out)",
                    report.properties, report.elements, report.left_out
                );
            }
            Err(e) => {
                eprintln!("rvt-ifc: properties not placed as Revit's exporter places them: {e:#}")
            }
        }
    }

    if args.no_base_quantities {
        let left_out = rvt::ifc::leave_out_base_quantities(&mut model);
        rvt::ifc::recount_exported(&mut diagnostics, &model);
        eprintln!("rvt-ifc: {left_out} base quantities left out");
    }

    // A reader that stopped at its work budget leaves out what it would have
    // read, and the export is still written, with this warning.
    if let Some(stops) = diagnostics.work_budget_stops() {
        eprintln!(
            "rvt-ifc: warning: {} reader(s) stopped at a work budget, so the export leaves out what they would have read:",
            stops.len()
        );
        for line in rvt::cli::work_budget_report(stops) {
            eprintln!("{line}");
        }
    }
    // the steps above may have read more streams than the export did.
    rvt::ifc::record_unread_streams(&mut diagnostics, rf.unread_streams());
    let unread = rf.unread_streams().len();
    if unread > 0 {
        eprintln!(
            "rvt-ifc: {unread} stream(s) not read whole, so the export is missing what they hold:"
        );
        for line in rvt::cli::unread_stream_report(rf.unread_streams()) {
            eprintln!("{line}");
        }
        if !args.allow_partial {
            if let Some(diagnostics_path) = &args.diagnostics {
                write_diagnostics_sidecar(diagnostics_path, &diagnostics, args.anonymize)?;
            }
            return Err(PartialRead.into());
        }
        eprintln!("rvt-ifc: writing the incomplete export (--allow-partial)");
    }

    if let Err(err) = quality_mode.validate(&diagnostics) {
        if let Some(diagnostics_path) = &args.diagnostics {
            write_diagnostics_sidecar(diagnostics_path, &diagnostics, args.anonymize)?;
        }
        anyhow::bail!("{err}");
    }

    warn_about_export_quality(&diagnostics);

    for note in rvt::convert::share_and_map(&mut rf, &mut model, &opts) {
        eprintln!("rvt-ifc: {note}");
    }

    // What IFCX carries beyond the IFC4 model, read while the file is open.
    let revit_elements = if args.ifcx.is_some() && !args.no_ifcx_revit_elements {
        rvt::control::stage("revit elements");
        let named_types: std::collections::BTreeSet<u64> = model
            .element_type_ids
            .values()
            .map(|&t| u64::from(t))
            .collect();
        rvt::ifc::ifcx_revit_elements::read(&mut rf, &named_types, model.georeference.as_ref())
            .unwrap_or_else(|e| {
                eprintln!("rvt-ifc: warning: IFCX written without the file's other elements: {e}");
                Vec::new()
            })
    } else {
        Vec::new()
    };
    // Everything below is written from the graph: free the file (its
    // inflated streams and memos) before it is built, and the model after.
    drop(rf);
    let georeference = model.georeference.clone();
    report.elements_by_category = rvt::ifc::element_table::count_by_category(&model);
    let untitled = model.project_name.is_none();
    rvt::control::stage("graph");
    // The model is the IFCX-shaped graph; the IFC4 text is derived from it.
    let mut graph = rvt::convert::graph(
        &model,
        &opts,
        args.ifcx.is_some() || args.frag.is_some() || args.usda.is_some() || args.usdc.is_some(),
    );
    drop(model);
    rvt::control::stage("write ifc");
    let timestamp = match args.ifc4_timestamp {
        Some(t) => Some(t),
        None => source_date_epoch()?,
    };
    // The witness observation reads the text back, and the IFC options edit
    // it whole; otherwise it is written as it is built, beside the output and
    // renamed when whole, so a failed or killed export never leaves a
    // truncated IFC under the output's name.
    let mut observed_step = None;
    let written = if args.observation.is_some() || !ifc_options.is_default() {
        // Built through the streaming writer, which refuses a model holding a
        // non-finite number rather than write a file no IFC reader opens.
        let mut bytes = Vec::new();
        rvt::convert::ifc4_step_to(&graph, timestamp, &mut bytes)?;
        let mut step = String::from_utf8(bytes)?;
        if !ifc_options.is_default() {
            rvt::control::stage("ifc options");
            let (edited, options_report) =
                rvt::ifc::ifc_options::apply(&step, &ifc_options, georeference.as_ref())
                    .map_err(|e| anyhow::anyhow!("{e}; no IFC written"))?;
            step = edited;
            let note = format!(
                "IFC options: {} property sets left out, {} elements kept and {} left out, {} instances no longer used dropped",
                options_report.property_sets_left_out,
                options_report.elements_kept,
                options_report.elements_left_out,
                options_report.instances_dropped
            );
            eprintln!("rvt-ifc: {note}");
            if !options_report.unmatched_categories.is_empty() {
                let warning = format!(
                    "warning: --ifc-categories matched no element for: {}",
                    options_report.unmatched_categories.join(", ")
                );
                eprintln!("rvt-ifc: {warning}");
                report.notes.push(warning);
            }
            report.notes.push(note);
            report.ifc_options = Some(options_report);
        }
        rvt::cli::write_output(&out_path, step.as_bytes())?;
        let written = step.len() as u64;
        if args.observation.is_some() {
            observed_step = Some(step);
        }
        written
    } else {
        let mut partial = out_path.clone().into_os_string();
        partial.push(".partial");
        let partial = PathBuf::from(partial);
        let result = (|| -> std::io::Result<u64> {
            let mut file = std::io::BufWriter::new(std::fs::File::create(&partial)?);
            let written = rvt::convert::ifc4_step_to(&graph, timestamp, &mut file)?;
            std::io::Write::flush(&mut file)?;
            file.into_inner()
                .map_err(std::io::IntoInnerError::into_error)?
                .sync_all()?;
            std::fs::rename(&partial, &out_path)?;
            Ok(written)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        result?
    };
    report.output("ifc", &out_path);
    eprintln!("rvt-ifc: wrote {written} bytes to {}", out_path.display());
    if let Some(frag_path) = &args.frag {
        rvt::control::stage("write frag");
        let (bytes, stats) = rvt::ifc::frag_writer::write_frag_graph_with(
            &graph,
            &rvt::ifc::gltf::DisplayOptions {
                rooms: args.frag_rooms,
                ..Default::default()
            },
        )?;
        rvt::cli::write_output(frag_path, &bytes)?;
        report.output("frag", frag_path);
        eprintln!(
            "rvt-ifc: wrote {} Fragments bytes to {}",
            bytes.len(),
            frag_path.display()
        );
        if let Some(p) = &args.frag_stats {
            rvt::cli::write_output(p, serde_json::to_vec_pretty(&stats)?)?;
        }
    }
    if let Some(usda_path) = &args.usda {
        rvt::control::stage("write usd");
        let text = write_usda_graph(&graph)?;
        rvt::cli::write_output(usda_path, text.as_bytes())?;
        report.output("usda", usda_path);
        eprintln!(
            "rvt-ifc: wrote {} USD bytes to {}",
            text.len(),
            usda_path.display()
        );
    }
    if let Some(usdc_path) = &args.usdc {
        let bytes = write_usdc_graph(&graph)?;
        rvt::cli::write_output(usdc_path, &bytes)?;
        report.output("usdc", usdc_path);
        eprintln!(
            "rvt-ifc: wrote {} USD binary bytes to {}",
            bytes.len(),
            usdc_path.display()
        );
    }
    if let Some(ifcx_path) = &args.ifcx {
        rvt::control::stage("write ifcx");
        // Last, so the .frag and USD files above are as before.
        graph.add_revit_elements(&revit_elements, args.ifcx_unused_types);
        // header time: --ifc4-timestamp, SOURCE_DATE_EPOCH, else the input file's own modified time
        let ifcx_time = timestamp.or_else(|| {
            std::fs::metadata(&args.input)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
        });
        // Written as it is built, not held whole.
        let (written, (diagnostics_layer, ledger)) =
            rvt::cli::write_output_with(ifcx_path, |out| {
                write_ifcx_graph_layers_to(&graph, ifcx_time, out)
            })?;
        report.output("ifcx", ifcx_path);
        eprintln!(
            "rvt-ifc: wrote {written} IFCX bytes to {}",
            ifcx_path.display()
        );
        if let Some(p) = &args.ifcx_diagnostics {
            rvt::cli::write_output(p, diagnostics_layer.as_bytes())?;
            eprintln!(
                "rvt-ifc: wrote {} IFCX diagnostics bytes to {}",
                diagnostics_layer.len(),
                p.display()
            );
        }
        if let Some(ledger_path) = &args.ifcx_ledger {
            rvt::cli::write_output(ledger_path, serde_json::to_vec_pretty(&ledger)?)?;
        }
    }
    for line in export_summary(
        &diagnostics,
        args.diagnostics.is_some(),
        is_family_file(&args.input),
    ) {
        eprintln!("{line}");
    }
    if let Some(diagnostics_path) = &args.diagnostics {
        write_diagnostics_sidecar(diagnostics_path, &diagnostics, args.anonymize)?;
        report.output("diagnostics", diagnostics_path);
    }
    if let Some(report_path) = &args.report {
        if unread > 0 {
            report.status = rvt::ifc::run_report::RunStatus::Partial;
        }
        report.seconds = started.elapsed().as_secs_f64();
        let mut d = diagnostics.clone();
        if args.anonymize {
            d.anonymize();
        }
        report.diagnostics = Some(d);
        rvt::cli::write_output(report_path, report.to_json())?;
        eprintln!("rvt-ifc: wrote run report to {}", report_path.display());
    }
    if let Some(observation_path) = &args.observation {
        write_observation(
            observation_path,
            &args.input,
            args.artifact_id.as_deref(),
            &diagnostics,
            observed_step.as_deref().unwrap_or_default(),
        )?;
    }
    rvt::control::stage("done");
    if untitled {
        eprintln!(
            "note: no project name extracted; output IFC uses \"Untitled\". \
             Title typically comes from PartAtom — this file may not have one."
        );
    }
    Ok(())
}

fn parse_guid_namespace(text: &str) -> Result<String, String> {
    rvt::ifc::validate_guid_namespace(text).map(|()| text.to_string())
}

fn convert_options_from_args(args: &Args) -> rvt::convert::ConvertOptions {
    rvt::convert::ConvertOptions {
        placeholder: args.placeholder,
        quality: ExportQualityMode::from(args.mode),
        convention: args.convention.into(),
        diagnostic_proxies: args.diagnostic_proxies,
        // real shapes are the default; the --no-* flags opt out.
        family_geometry: !args.placeholder && !args.no_family_geometry,
        family_first: args.family_first,
        saved_meshes: !args.placeholder && !args.no_saved_meshes,
        no_saved_meshes: args.no_saved_meshes,
        saved_materials: !args.placeholder && !args.no_saved_materials,
        proxy_shapes: !args.placeholder && !args.no_proxy_shapes,
        per_element_meshes: args.per_element_meshes,
        revit_property_sets: !args.placeholder && !args.no_revit_property_sets,
        epsg: args.epsg,
        guid_namespace: args.guid_namespace.clone(),
        walker_limits: walker_limits_from_args(args),
        element_provenance: args.element_provenance,
        export_decisions: args.export_decisions,
        family_extrusions: args.family_extrusions,
    }
}

fn walker_limits_from_args(args: &Args) -> WalkerLimits {
    let defaults = WalkerLimits::default();
    WalkerLimits {
        max_scan_bytes: args.max_walker_scan_bytes.unwrap_or(if args.large_model {
            rvt::cli::LARGE_MODEL_MAX_INFLATE_BYTES
        } else {
            defaults.max_scan_bytes
        }),
        max_candidates: args
            .max_walker_candidates
            .unwrap_or(defaults.max_candidates),
        max_trial_offsets: args
            .max_walker_trial_offsets
            .unwrap_or(defaults.max_trial_offsets),
        max_per_record_decode_bytes: args
            .max_walker_record_decode_bytes
            .unwrap_or(defaults.max_per_record_decode_bytes),
        max_container_records: args
            .max_walker_container_records
            .unwrap_or(defaults.max_container_records),
    }
}

fn write_diagnostics_sidecar(
    path: &std::path::Path,
    diagnostics: &ExportDiagnostics,
    anonymize: bool,
) -> anyhow::Result<()> {
    let json = if anonymize {
        let mut anonymized = diagnostics.clone();
        anonymized.anonymize();
        serde_json::to_vec_pretty(&anonymized)?
    } else {
        serde_json::to_vec_pretty(diagnostics)?
    };
    rvt::cli::write_output(path, &json)?;
    eprintln!(
        "rvt-ifc: wrote diagnostics {} bytes to {}",
        json.len(),
        path.display()
    );
    Ok(())
}

/// witness observation: canonical JSON is serde_json's default
/// output — keys sorted (no `preserve_order`), no whitespace — which is
/// byte-identical to the Python witnesses' `sort_keys` + compact separators,
/// so the payload hash is comparable across languages.
fn write_observation(
    path: &std::path::Path,
    input: &std::path::Path,
    artifact_id: Option<&str>,
    diagnostics: &ExportDiagnostics,
    step: &str,
) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(input)?;
    let input_hash = lower_hex(&Sha256::digest(&bytes));
    // entity_counts is what an IFC reader would see in the STEP this run
    // wrote — every `#n=IFCTYPE(` constructor, not just the building-element
    // histogram in the diagnostics (which omits units, contexts, rels).
    let mut entity_counts = std::collections::BTreeMap::<String, usize>::new();
    for line in step.lines() {
        let ifc_type = line
            .strip_prefix('#')
            .and_then(|rest| rest.split_once('='))
            .and_then(|(_, after_eq)| after_eq.trim_start().split_once('('))
            .map(|(ifc_type, _)| ifc_type);
        if let Some(ifc_type) = ifc_type {
            *entity_counts.entry(ifc_type.to_string()).or_default() += 1;
        }
    }
    let payload = serde_json::json!({
        "entity_counts": entity_counts,
        "relations": relation_pairs(step),
        "storeys": storey_set(step),
        "exported_building_elements": diagnostics.exported.by_ifc_type,
        "storey_count": diagnostics.exported.storey_count,
        "material_count": diagnostics.exported.material_count,
        "building_elements_with_geometry": diagnostics.exported.building_elements_with_geometry,
    });
    let canonical = serde_json::to_string(&payload)?;
    let observation = serde_json::json!({
        "schema_version": "1.1.0",
        "witness_id": "rvt-open",
        "witness_version": env!("CARGO_PKG_VERSION"),
        "artifact_id": artifact_id,
        "input_role": "source",
        "input_file": input.file_name().and_then(|n| n.to_str()),
        "input_hash_sha256": input_hash,
        "deterministic": true,
        "semantic_surface_covered": ["entity_counts", "relations", "storeys"],
        "observation": payload,
        "observation_hash_sha256": lower_hex(&Sha256::digest(canonical.as_bytes())),
        "unsupported_entities": diagnostics.unsupported_features,
        "warnings": diagnostics.warnings,
    });
    let mut json = serde_json::to_vec_pretty(&observation)?;
    json.push(b'\n');
    rvt::cli::write_output(path, &json)?;
    eprintln!(
        "rvt-ifc: wrote observation ({} bytes) to {}",
        json.len(),
        path.display()
    );
    Ok(())
}

/// Attribute index of `Tag` on every `IfcElement` subtype in IFC4.
///
/// `IfcElement` adds exactly one attribute to `IfcProduct`'s seven
/// (`GlobalId`, `OwnerHistory`, `Name`, `Description`, `ObjectType`,
/// `ObjectPlacement`, `Representation`), so the index is the same for
/// `IfcWall`, `IfcDoor`, `IfcWindow` and every other element type.
const IFC_ELEMENT_TAG_INDEX: usize = 7;

/// One STEP instance: its type keyword and its top-level attributes.
struct StepInstance<'a> {
    ifc_type: &'a str,
    attributes: Vec<&'a str>,
}

/// Split a STEP attribute list on top-level commas.
///
/// Commas inside a nested list or a quoted string do not separate
/// attributes. STEP escapes an apostrophe by doubling it, which this
/// scanner handles for free: the pair closes and immediately reopens
/// the string with nothing between the two quotes.
fn split_attributes(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut start = 0usize;
    for (index, ch) in args.char_indices() {
        match ch {
            '\'' => in_string = !in_string,
            '(' if !in_string => depth += 1,
            ')' if !in_string => depth = depth.saturating_sub(1),
            ',' if !in_string && depth == 0 => {
                out.push(args[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(args[start..].trim());
    out
}

/// Index the STEP data section by instance id. One instance per line
/// — the same assumption the `entity_counts` scan above makes, and
/// the shape `ifc::write_step` emits.
fn parse_step_instances(step: &str) -> std::collections::BTreeMap<u32, StepInstance<'_>> {
    let mut out = std::collections::BTreeMap::new();
    for line in step.lines() {
        let Some(rest) = line.strip_prefix('#') else {
            continue;
        };
        let Some((id_text, after_eq)) = rest.split_once('=') else {
            continue;
        };
        let Ok(id) = id_text.trim().parse::<u32>() else {
            continue;
        };
        let after_eq = after_eq.trim_start();
        let Some((ifc_type, args)) = after_eq.split_once('(') else {
            continue;
        };
        let Some(args) = args.trim_end().strip_suffix(';').and_then(|a| {
            let a = a.trim_end();
            a.strip_suffix(')')
        }) else {
            continue;
        };
        out.insert(
            id,
            StepInstance {
                ifc_type: ifc_type.trim(),
                attributes: split_attributes(args),
            },
        );
    }
    out
}

/// Resolve `#123` to its instance id.
fn entity_ref(attribute: &str) -> Option<u32> {
    attribute.trim().strip_prefix('#')?.trim().parse().ok()
}

/// Decode a STEP string literal, inverting `ifc::step_writer::escape`.
///
/// Returns `None` for `$` or anything that is not a quoted literal —
/// an unset `Tag` is not a string.
fn step_string(attribute: &str) -> Option<String> {
    let body = attribute.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            // Doubled apostrophe — consume the second half.
            if chars.peek() == Some(&'\'') {
                chars.next();
            }
            out.push('\'');
            continue;
        }
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('X') | Some('x') => {
                let width = match chars.peek() {
                    Some('\\') => {
                        chars.next();
                        2
                    }
                    Some('2') => {
                        chars.next();
                        chars.next();
                        4
                    }
                    Some('4') => {
                        chars.next();
                        chars.next();
                        8
                    }
                    _ => return None,
                };
                loop {
                    let mut digits = String::with_capacity(width);
                    for _ in 0..width {
                        digits.push(chars.next()?);
                    }
                    let code = u32::from_str_radix(&digits, 16).ok()?;
                    out.push(char::from_u32(code)?);
                    if width == 2 {
                        break;
                    }
                    // `\X2\` / `\X4\` runs continue until the `\X0\`
                    // terminator the writer always emits.
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                        chars.next();
                        chars.next();
                        chars.next();
                        break;
                    }
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// `IfcRelFillsElement` host/filling `Tag` pairs, canonically sorted.
///
/// The chain is Revit's own: `IfcRelVoidsElement` binds an opening to
/// the element it voids, `IfcRelFillsElement` binds that opening to
/// the element that fills it, so the pair `[host Tag, filling Tag]`
/// is the door/window ↔ host-wall relation as an IFC reader sees it
/// (field class *relation pair sets*).
///
/// A `Tag` that is unset, or an opening with no `IfcRelVoidsElement`,
/// yields an empty string rather than dropping the pair: a missing
/// half must surface as a disagreement, never as a silent omission.
/// Duplicates are kept, so the value is a sorted multiset.
fn relation_pairs(step: &str) -> std::collections::BTreeMap<String, Vec<Vec<String>>> {
    let instances = parse_step_instances(step);
    let tag_of = |id: Option<u32>| -> String {
        id.and_then(|id| instances.get(&id))
            .and_then(|instance| instance.attributes.get(IFC_ELEMENT_TAG_INDEX))
            .and_then(|attribute| step_string(attribute))
            .unwrap_or_default()
    };
    let mut voided_by: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    for instance in instances.values() {
        if !instance.ifc_type.eq_ignore_ascii_case("IFCRELVOIDSELEMENT") {
            continue;
        }
        // (GlobalId, OwnerHistory, Name, Description,
        //  RelatingBuildingElement, RelatedOpeningElement)
        if let (Some(host), Some(opening)) = (
            instance.attributes.get(4).and_then(|a| entity_ref(a)),
            instance.attributes.get(5).and_then(|a| entity_ref(a)),
        ) {
            voided_by.insert(opening, host);
        }
    }
    let mut pairs: Vec<Vec<String>> = Vec::new();
    for instance in instances.values() {
        if !instance.ifc_type.eq_ignore_ascii_case("IFCRELFILLSELEMENT") {
            continue;
        }
        // (GlobalId, OwnerHistory, Name, Description,
        //  RelatingOpeningElement, RelatedBuildingElement)
        let opening = instance.attributes.get(4).and_then(|a| entity_ref(a));
        let filling = instance.attributes.get(5).and_then(|a| entity_ref(a));
        let host = opening.and_then(|id| voided_by.get(&id).copied());
        pairs.push(vec![tag_of(host), tag_of(filling)]);
    }
    pairs.sort();
    let mut out = std::collections::BTreeMap::new();
    out.insert("IFCRELFILLSELEMENT".to_string(), pairs);
    out
}

/// Attribute index of `Name` on `IfcBuildingStorey` (`IfcRoot` +2).
const IFC_BUILDING_STOREY_NAME_INDEX: usize = 2;

/// Attribute index of `Elevation` on `IfcBuildingStorey`: the tenth and
/// last attribute of the IFC4 entity.
const IFC_BUILDING_STOREY_ELEVATION_INDEX: usize = 9;

/// Metres in one international foot, exactly.
const METRES_PER_FOOT: f64 = 0.3048;

/// Parse a STEP `REAL`/`INTEGER` literal.
fn step_number(attribute: &str) -> Option<f64> {
    attribute.trim().parse::<f64>().ok()
}

/// Multiplier that takes this model's declared `LENGTHUNIT` to metres.
///
/// Walks `IfcProject.UnitsInContext` → `IfcUnitAssignment.Units` and
/// resolves the first length unit: an `IfcSIUnit` contributes its SI
/// prefix, an `IfcConversionBasedUnit` its
/// `IfcMeasureWithUnit.ValueComponent` times the SI unit that measure
/// is expressed in. `None` when the model declares no length unit —
/// the caller must not guess, because a wrong scale silently moves
/// every storey.
fn length_unit_scale_to_metres(
    instances: &std::collections::BTreeMap<u32, StepInstance<'_>>,
) -> Option<f64> {
    fn si_prefix_multiplier(prefix: &str) -> f64 {
        match prefix
            .trim()
            .trim_matches('.')
            .to_ascii_uppercase()
            .as_str()
        {
            "EXA" => 1e18,
            "PETA" => 1e15,
            "TERA" => 1e12,
            "GIGA" => 1e9,
            "MEGA" => 1e6,
            "KILO" => 1e3,
            "HECTO" => 1e2,
            "DECA" => 1e1,
            "DECI" => 1e-1,
            "CENTI" => 1e-2,
            "MILLI" => 1e-3,
            "MICRO" => 1e-6,
            "NANO" => 1e-9,
            "PICO" => 1e-12,
            "FEMTO" => 1e-15,
            "ATTO" => 1e-18,
            _ => 1.0,
        }
    }
    fn enum_value(attribute: &str) -> String {
        attribute.trim().trim_matches('.').to_ascii_uppercase()
    }
    fn si_scale(instance: &StepInstance<'_>) -> Option<f64> {
        if !instance.ifc_type.eq_ignore_ascii_case("IFCSIUNIT") {
            return None;
        }
        Some(match instance.attributes.get(2) {
            Some(prefix) if prefix.trim() != "$" => si_prefix_multiplier(prefix),
            _ => 1.0,
        })
    }

    let project = instances
        .values()
        .find(|instance| instance.ifc_type.eq_ignore_ascii_case("IFCPROJECT"))?;
    let assignment = instances.get(&entity_ref(project.attributes.get(8)?)?)?;
    let units = assignment.attributes.first()?.trim();
    let units = units.strip_prefix('(')?.strip_suffix(')')?;
    for reference in split_attributes(units) {
        let Some(unit) = entity_ref(reference).and_then(|id| instances.get(&id)) else {
            continue;
        };
        let Some(unit_type) = unit.attributes.get(1).map(|a| enum_value(a)) else {
            continue;
        };
        if unit_type != "LENGTHUNIT" {
            continue;
        }
        if let Some(scale) = si_scale(unit) {
            return Some(scale);
        }
        if unit.ifc_type.eq_ignore_ascii_case("IFCCONVERSIONBASEDUNIT") {
            let measure = instances.get(&entity_ref(unit.attributes.get(3)?)?)?;
            let value = measure.attributes.first()?.trim();
            let ratio = value
                .split_once('(')
                .and_then(|(_, rest)| rest.strip_suffix(')'))
                .and_then(step_number)
                .or_else(|| step_number(value))?;
            let base = instances.get(&entity_ref(measure.attributes.get(1)?)?)?;
            return Some(ratio * si_scale(base)?);
        }
    }
    None
}

/// `IfcBuildingStorey` `[Name, Elevation]` pairs, canonically sorted.
///
/// The elevation is converted from the model's declared `LENGTHUNIT`
/// to feet and rendered with six decimals, so the field compares
/// across witnesses whose files declare different units — Revit's own
/// export of this corpus declares `FOOT` while rvt-open writes `METRE`
/// (field class *storey sets*). The name is the
/// decoded `Name` string verbatim; an unset `Name` contributes an
/// empty string rather than dropping the storey, so a missing half
/// surfaces as a disagreement and never as a silent omission.
///
/// An empty list is returned when the model declares no length unit:
/// a storey whose unit is unknown is not a measurement.
fn storey_set(step: &str) -> std::collections::BTreeMap<String, Vec<Vec<String>>> {
    let instances = parse_step_instances(step);
    let mut storeys: Vec<Vec<String>> = Vec::new();
    if let Some(scale) = length_unit_scale_to_metres(&instances) {
        for instance in instances.values() {
            if !instance.ifc_type.eq_ignore_ascii_case("IFCBUILDINGSTOREY") {
                continue;
            }
            let name = instance
                .attributes
                .get(IFC_BUILDING_STOREY_NAME_INDEX)
                .and_then(|attribute| step_string(attribute))
                .unwrap_or_default();
            let elevation = instance
                .attributes
                .get(IFC_BUILDING_STOREY_ELEVATION_INDEX)
                .and_then(|attribute| step_number(attribute));
            let feet = match elevation {
                Some(value) => format_elevation_feet(value * scale / METRES_PER_FOOT),
                None => String::new(),
            };
            storeys.push(vec![name, feet]);
        }
    }
    storeys.sort();
    let mut out = std::collections::BTreeMap::new();
    out.insert("IFCBUILDINGSTOREY".to_string(), storeys);
    out
}

/// Render an elevation in feet at 1e-6 ft, as a string.
///
/// A string, not a JSON number, because the canonical form
/// is defined over integers and strings only — two
/// runtimes must not be trusted to print the same `f64` the same way.
/// `-0` is normalised to `0`.
fn format_elevation_feet(feet: f64) -> String {
    if !feet.is_finite() {
        return String::new();
    }
    let rendered = format!("{feet:.6}");
    if rendered == "-0.000000" {
        return "0.000000".to_string();
    }
    rendered
}

/// How many IFC types the summary names before "and N more".
const SUMMARY_TYPES: usize = 6;

/// Whether `path` names a family (`.rfa`) or a family template (`.rft`), whose
/// elements are read from `Global/Latest` and not from the project element
/// records.
fn is_family_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["rfa", "rft"]
                .iter()
                .any(|family| extension.eq_ignore_ascii_case(family))
        })
}

/// A few lines saying what the IFC holds: building elements by IFC type,
/// what was left out on purpose, and the readiness level.
///
/// `family` is whether the input is a family file, for which the element
/// records of a project say nothing.
fn export_summary(diagnostics: &ExportDiagnostics, has_sidecar: bool, family: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let exported = &diagnostics.exported;
    if exported.building_elements > 0 {
        let carried = match exported.building_elements_carried_by_parts {
            0 => String::new(),
            n => format!(" and {n} carried by their parts"),
        };
        lines.push(format!(
            "rvt-ifc: {} building element(s), {} with geometry{carried}, on {} storey(s)",
            exported.building_elements,
            exported.building_elements_with_geometry,
            exported.storey_count
        ));
        let mut types: Vec<(&String, &usize)> = exported.by_ifc_type.iter().collect();
        types.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        let mut named: Vec<String> = types
            .iter()
            .take(SUMMARY_TYPES)
            .map(|(name, count)| format!("{count} {}", ifc_type_display(name)))
            .collect();
        if types.len() > SUMMARY_TYPES {
            named.push(format!("and {} more type(s)", types.len() - SUMMARY_TYPES));
        }
        lines.push(format!("  {}", named.join(", ")));
        // say how much of the model is a stand-in box.
        if exported.bounding_box_bodies > 0 {
            lines.push(format!(
                "  {} of the bodies are their element record's bounding box: placed and sized, shape not decoded",
                exported.bounding_box_bodies
            ));
        }
        // Say how many spaces are a box, as their area is the box's.
        if exported.bounding_box_spaces > 0 {
            lines.push(format!(
                "  {} of {} spaces are their record's box: the area of their body is the box's, not the space's (SpaceOutlineSource record_box)",
                exported.bounding_box_spaces,
                exported.by_ifc_type.get("IFCSPACE").copied().unwrap_or(0)
            ));
        }
        // Rooms read from their stored solid by a rule that is not
        // the exact one.
        if exported.resolved_outline_spaces > 0 {
            lines.push(format!(
                "  {} rooms take an outline read from the faces their stored solid lists, where the exact walk did not close them (SpaceOutlineSource stored_solid_faces)",
                exported.resolved_outline_spaces
            ));
        }
        // rooms read from their solid's edges.
        if exported.edge_outline_spaces > 0 {
            lines.push(format!(
                "  {} rooms take the outline of their stored solid's floor, read from its edges where neither the walk nor the span closed it (SpaceOutlineSource stored_solid_edges)",
                exported.edge_outline_spaces
            ));
        }
        if exported.approximate_outline_spaces > 0 {
            lines.push(format!(
                "  {} rooms take an approximate outline, a column that overlaps a wall taken off it or up to 0.02 ft short of their record box on a side: the area is within about 1% of the space's, not 0.1% (SpaceOutlineSource stored_solid_approximate)",
                exported.approximate_outline_spaces
            ));
        }
    }
    let left_out: Vec<String> = diagnostics
        .skipped
        .iter()
        .filter_map(|item| {
            let what = match item.reason.as_str() {
                "element_record_without_volume" => "without a 3D body",
                "element_record_in_non_primary_design_option" => "in non-primary design options",
                "element_record_not_in_export_phase" => "not in the export phase",
                "element_record_empty_curtain_panel" => "empty curtain panels",
                "room_not_in_export_phase" => "rooms in a phase other than the export phase",
                "element_record_family_type_draws_nothing" => {
                    "plumbing fixtures whose type draws nothing visible"
                }
                "element_record_family_space_without_body" => {
                    "family instances written as spaces whose family draws no model form"
                }
                _ => return None,
            };
            Some(format!("{} {what}", item.count))
        })
        .collect();
    if !left_out.is_empty() {
        lines.push(format!(
            "  left out as Revit's own export leaves them out: {}",
            left_out.join(", ")
        ));
    }
    // A Revit 2008 to 2023 project none of whose element records was
    // written says where they stopped, on a line of its own (it can be the only
    // line, when nothing else was exported). A family file reads its elements
    // from `Global/Latest`, so for one it says nothing.
    if let Some(stages) = diagnostics
        .decoded
        .element_record_stages
        .as_ref()
        .filter(|_| !family)
    {
        if let Some(stopped) = &stages.stopped_at {
            lines.push(format!(
                "rvt-ifc: no element record of this Revit {} layout was written: {} marker(s) found, they stopped at {stopped} (decoded.element_record_stages in the diagnostics)",
                stages.layout_release, stages.total.markers
            ));
        }
    }
    // A scaffold-only export already warned with the same advice.
    let hint = if has_sidecar || diagnostics.confidence.level == "scaffold" {
        ""
    } else {
        ". Add --diagnostics <path> for the full report"
    };
    lines.push(format!(
        "rvt-ifc: readiness {} (score {:.2}){hint}",
        diagnostics.confidence.level, diagnostics.confidence.score
    ));
    lines
}

/// `IFCWALL` as `IfcWall`, the spelling IFC documentation and viewers use.
/// A type outside the list keeps the STEP spelling.
fn ifc_type_display(upper: &str) -> String {
    const NAMES: &[&str] = &[
        "IfcAirTerminal",
        "IfcAlarm",
        "IfcBeam",
        "IfcBuildingElementProxy",
        "IfcCableCarrierFitting",
        "IfcCableCarrierSegment",
        "IfcColumn",
        "IfcCovering",
        "IfcCurtainWall",
        "IfcDoor",
        "IfcDuctFitting",
        "IfcDuctSegment",
        "IfcElectricAppliance",
        "IfcElementAssembly",
        "IfcFlowController",
        "IfcFooting",
        "IfcFurniture",
        "IfcLightFixture",
        "IfcMember",
        "IfcPipeFitting",
        "IfcPipeSegment",
        "IfcPlate",
        "IfcRailing",
        "IfcRamp",
        "IfcReinforcingBar",
        "IfcRoof",
        "IfcSanitaryTerminal",
        "IfcShadingDevice",
        "IfcSlab",
        "IfcSpace",
        "IfcStair",
        "IfcStairFlight",
        "IfcSystemFurnitureElement",
        "IfcTransportElement",
        "IfcWall",
        "IfcWindow",
    ];
    NAMES
        .iter()
        .find(|name| name.eq_ignore_ascii_case(upper))
        .map_or_else(|| upper.to_string(), |name| (*name).to_string())
}

fn warn_about_export_quality(diagnostics: &ExportDiagnostics) {
    if diagnostics.confidence.level == "scaffold" {
        eprintln!(
            "warning: export confidence is scaffold-only; no validated building elements were exported. \
             Re-run with `--diagnostics <path>` for a shareable readiness report."
        );
    }
    let limit_hits = diagnostics.confidence.resource_limit_hits;
    if limit_hits > 0 {
        eprintln!(
            "warning: the IFC may be incomplete: {limit_hits} resource limit(s) stopped or \
             truncated reading this file (see the `resource-limit:` warnings in --diagnostics)."
        );
    }
    let skipped = diagnostics.confidence.skipped_members;
    if skipped > 0 {
        eprintln!(
            "warning: the IFC may be incomplete: {skipped} gzip member region(s) or stream(s) \
             would not decode and were skipped (see the `skipped-member:` warnings in --diagnostics)."
        );
    }
    let unexported = diagnostics.confidence.unexported_element_records;
    if unexported > 0 {
        eprintln!(
            "warning: {unexported} element record(s) belong to ElementIds the file's element table \
             does not declare and were not exported. On the one model compared with \
             Revit's export of the same save, such records were elements deleted since the file's \
             last Save As; if nothing was deleted, the IFC is likely incomplete."
        );
    }
}

/// `bytes` as lowercase hex, two digits a byte.
fn lower_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
