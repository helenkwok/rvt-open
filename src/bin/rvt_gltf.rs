//! `rvt-gltf` — convert a Revit file to a glTF 2.0 binary (.glb).
//!
//! The command-line front end of `rvt::ifc::gltf`. Reads `src`, runs the
//! `RvtDocExporter` to produce an `IfcModel` (project metadata +
//! whatever per-element info the walker can resolve), then emits
//! the glb via `ifc::gltf::model_to_glb`.
//!
//! Usage:
//!
//! ```bash
//! rvt-gltf model.rvt                    # writes model.glb next to the input
//! rvt-gltf model.rvt -o out/model.glb --verbose
//! rvt-gltf --src model.rvt --dst model.glb   # older spelling, still accepted
//! rvt-gltf model.rvt --format pdf3d          # writes model.pdf, a 3D PDF
//! rvt-gltf model.rvt --format obj            # writes model.obj and model.mtl
//! ```
//!
//! `--format pdf3d` writes the same meshes (one node per element, no
//! instancing extension, no edges) as U3D in a one-page PDF 3D annotation
//! (`rvt::ifc::pdf3d`).

use clap::Parser;
use rvt::RevitFile;
use rvt::ifc::gltf::{DisplayOptions, GltfOptions, model_to_glb_with};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "rvt-gltf",
    version,
    about = "Convert a Revit (.rvt, .rfa, .rte, .rft) file to a glTF 2.0 binary (.glb)",
    after_help = "Examples:\n  \
        rvt-gltf model.rvt                      writes model.glb next to the input\n  \
        rvt-gltf model.rvt -o viewer/model.glb --verbose\n  \
        rvt-gltf model.rvt --format pdf3d       writes model.pdf, a 3D PDF (U3D)\n  \
        rvt-gltf model.rvt --format obj         writes model.obj and its materials, model.mtl\n\n\
        Exit status 3: a resource limit kept part of the file from being read, so\n\
         nothing was written. Re-run with --large-model, or --allow-partial to write\n\
         it anyway. A reader that stopped at its work budget is a warning: the output\n\
         is written."
)]
struct Cli {
    /// Revit file to convert (.rvt / .rfa / .rte / .rft).
    #[arg(
        value_name = "INPUT",
        required_unless_present = "src",
        conflicts_with = "src"
    )]
    input: Option<PathBuf>,

    /// Output path. Default: the input path with a `.glb` extension (`.pdf`
    /// for `--format pdf3d`).
    #[arg(short = 'o', long = "output", alias = "dst", short_alias = 'd')]
    output: Option<PathBuf>,

    /// Older spelling of INPUT (`--src model.rvt`), kept for scripts.
    #[arg(short = 's', long = "src", hide = true)]
    src: Option<PathBuf>,

    /// What to write: `glb` (glTF 2.0 binary), `pdf3d` (a one-page PDF
    /// whose 3D annotation holds the same meshes as U3D, with a flat poster
    /// for viewers that show no 3D) or `obj` (Wavefront OBJ with a `.mtl`
    /// of the colours beside it, one group per element named with its
    /// ElementId).
    #[arg(long, value_enum, default_value_t = Format::Glb)]
    format: Format,

    /// Print element count + output size on success.
    #[arg(long)]
    verbose: bool,

    /// Draw every element whose body would be its record's bounding box
    /// (furniture, fixtures, equipment and the rest) from the mesh Revit
    /// saved for display, on Revit 2014 to 2027 (on by default; this flag is
    /// kept for old command lines).
    #[arg(long, conflicts_with = "no_saved_meshes")]
    saved_meshes: bool,

    /// Keep every element as its record's bounding box, and the walls that
    /// embed a curtain wall as their record's body, instead of the mesh Revit
    /// saved for them.
    #[arg(long)]
    no_saved_meshes: bool,

    /// on by default (kept for old command lines).
    #[arg(long, conflicts_with = "no_family_geometry")]
    family_geometry: bool,

    /// Keep bounding-box bodies the saved graphics leave, instead of the
    /// family document's solids.
    #[arg(long)]
    no_family_geometry: bool,

    /// on by default (kept for old command lines).
    #[arg(long, conflicts_with = "no_saved_materials")]
    saved_materials: bool,

    /// Leave elements without a material as they are.
    #[arg(long)]
    no_saved_materials: bool,

    /// Keep trees, people and cars that no real shape decodes as their record's
    /// bounding box instead of an approximate proxy shape.
    #[arg(long)]
    no_proxy_shapes: bool,

    /// Draw room and space volumes in the default scene. Off (as Revit's 3D
    /// views), they sit under a `rooms` node only the glb's second scene,
    /// `with rooms`, shows.
    #[arg(long)]
    rooms: bool,

    /// Leave out the feature edges (black `LINES`, as Revit's Shaded with
    /// Show Edges).
    #[arg(long)]
    no_edges: bool,

    /// Print progress to stderr as the conversion goes
    /// (`progress: <stage> <done>/<total>`, as `rvt-elements --progress`).
    #[arg(long)]
    progress: bool,

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

    /// Write the glb even when a resource limit kept part of the file from
    /// being read.
    #[arg(long)]
    allow_partial: bool,

    /// a node per element (the mesh still shared) instead of one
    /// `EXT_mesh_gpu_instancing` node per repeated type mesh.
    #[arg(long)]
    no_instancing: bool,

    /// leave NORMAL out of all-flat meshes (glTF clients compute
    /// flat normals; three.js GLTFLoader does).
    #[arg(long)]
    omit_flat_normals: bool,

    /// compress vertex and index data losslessly with
    /// `EXT_meshopt_compression` (three.js: `setMeshoptDecoder`).
    #[arg(long)]
    meshopt: bool,
}

/// The output format.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Glb,
    Pdf3d,
    Obj,
}

/// Why a conversion stopped.
enum Failure {
    /// Exit status 1.
    Error(String),
    /// A resource limit kept part of the input from being read and
    /// `--allow-partial` was not given: exit status 3.
    PartialRead,
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::Error(message)
    }
}

fn main() -> ExitCode {
    rvt::cli::exit_quietly_on_broken_pipe();
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Error(e)) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
        Err(Failure::PartialRead) => {
            eprintln!("error: nothing written: part of the input was not read (see above)");
            ExitCode::from(3)
        }
    }
}

/// Input from INPUT or the legacy `--src`; output from `-o` or the input
/// path with `extension` swapped in. Refuses to overwrite the input.
fn resolve_paths(
    input: Option<&Path>,
    src: Option<&Path>,
    output: Option<&Path>,
    extension: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let input = input.or(src).ok_or("no input file given")?.to_path_buf();
    let output = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| input.with_extension(extension));
    if rvt::cli::same_file(&output, &input) {
        return Err(format!(
            "refusing to overwrite the input {}; pass -o <path>",
            input.display()
        ));
    }
    Ok((input, output))
}

fn run(cli: &Cli) -> Result<(), Failure> {
    let (input, output) = resolve_paths(
        cli.input.as_deref(),
        cli.src.as_deref(),
        cli.output.as_deref(),
        match cli.format {
            Format::Glb => "glb",
            Format::Pdf3d => "pdf",
            Format::Obj => "obj",
        },
    )?;
    // Open errors from the library already name the file.
    let limits =
        rvt::cli::open_limits(cli.max_stream_bytes, cli.max_inflate_bytes, cli.large_model);
    rvt::cli::log_stage_memory();
    let mut rf = RevitFile::open_with_limits(&input, limits).map_err(|e| e.to_string())?;
    rvt::control::stage("export");
    let mut walker_limits = rvt::walker::WalkerLimits::default();
    if cli.large_model {
        walker_limits.max_scan_bytes = rvt::cli::LARGE_MODEL_MAX_INFLATE_BYTES;
    }
    let control = if cli.progress {
        rvt::control::WalkerControl::new().with_progress(|event| {
            let total = event.total.map(|t| format!("/{t}")).unwrap_or_default();
            eprintln!("progress: {} {}{total}", event.stage.as_str(), event.done);
        })
    } else {
        rvt::control::WalkerControl::new()
    };
    let (mut model, notes) = rvt::convert::gltf_model(
        &mut rf,
        &rvt::convert::GltfModelOptions {
            saved_meshes: !cli.no_saved_meshes,
            family_geometry: !cli.no_family_geometry,
            saved_materials: !cli.no_saved_materials,
            proxy_shapes: !cli.no_proxy_shapes,
            walker_limits,
        },
        &control,
    )
    .map_err(|e| format!("export {}: {e}", input.display()))?;
    for note in notes {
        if cli.verbose || !note.starts_with("openings ") {
            eprintln!("rvt-gltf: {note}");
        }
    }
    // A reader that stopped at its work budget leaves out what it would have
    // read, and the model is still written, with this warning.
    let stops = rf.work_budget_stops();
    if !stops.is_empty() {
        eprintln!(
            "rvt-gltf: warning: {} reader(s) stopped at a work budget, so the model leaves out what they would have read:",
            stops.len()
        );
        for line in rvt::cli::work_budget_report(&stops) {
            eprintln!("{line}");
        }
    }
    let unread = rf.unread_streams().len();
    if unread > 0 {
        eprintln!(
            "rvt-gltf: {unread} stream(s) not read whole, so the model is missing what they hold:"
        );
        for line in rvt::cli::unread_stream_report(rf.unread_streams()) {
            eprintln!("{line}");
        }
        if !cli.allow_partial {
            return Err(Failure::PartialRead);
        }
        eprintln!("rvt-gltf: writing the incomplete model (--allow-partial)");
    }
    // The rest is written from the model: free the file first.
    drop(rf);
    rvt::control::stage("shared shapes");
    // Instances of one type draw one mesh.
    rvt::ifc::body_geometry::share_shapes_by_convention(&mut model);
    rvt::control::stage("write glb");
    if cli.format == Format::Obj {
        let mtl_path = output.with_extension("mtl");
        let mtl_name = mtl_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model.mtl".into());
        let (obj, mtl) = rvt::ifc::obj::model_to_obj(&model, &mtl_name);
        rvt::cli::write_output(&output, obj.as_bytes())
            .map_err(|e| format!("write {}: {e}", output.display()))?;
        rvt::cli::write_output(&mtl_path, mtl.as_bytes())
            .map_err(|e| format!("write {}: {e}", mtl_path.display()))?;
        rvt::control::stage("done");
        if cli.verbose {
            println!(
                "rvt-gltf: wrote {} bytes to {} and {}",
                obj.len(),
                output.display(),
                mtl_path.display()
            );
        }
        return Ok(());
    }
    let glb = if cli.format == Format::Pdf3d {
        rvt::ifc::pdf3d::model_to_pdf3d(&model)
    } else {
        model_to_glb_with(
            &model,
            &GltfOptions::new(!cli.no_instancing, cli.omit_flat_normals, cli.meshopt).with_display(
                DisplayOptions {
                    rooms: cli.rooms,
                    edges: !cli.no_edges,
                },
            ),
        )
    };
    rvt::cli::write_output(&output, &glb)
        .map_err(|e| format!("write {}: {e}", output.display()))?;
    rvt::control::stage("done");
    if cli.verbose {
        println!(
            "rvt-gltf: wrote {} bytes ({} building elements) to {}",
            glb.len(),
            model
                .entities
                .iter()
                .filter(|e| matches!(e, rvt::ifc::entities::IfcEntity::BuildingElement { .. }))
                .count(),
            output.display()
        );
    }
    Ok(())
}
