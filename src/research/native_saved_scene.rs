//! Current native saved-graphics extraction with explicit shared-symbol closure.
use crate::{
    RevitFile,
    control::{Stage, WalkerControl},
    native_document::{self, Options, Record, Summary},
    native_saved_mesh::{self, GraphicsMeshes},
};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Serialize)]
pub struct SavedElement {
    pub id: u64,
    pub identity: crate::native_index::Identity,
    pub source: serde_json::Value,
    pub referenced_graph_sources: BTreeMap<u64, serde_json::Value>,
    pub status: String,
    pub meshes: GraphicsMeshes,
    pub render_materials: BTreeMap<usize, crate::native_saved_materials::RenderMaterial>,
    pub unresolved_material_primitives: Vec<usize>,
}
#[derive(Debug, Serialize)]
pub struct SavedScene {
    pub source_sha256: Option<String>,
    pub format: &'static str,
    pub units: &'static str,
    pub coordinate_frame: &'static str,
    pub complete_document_geometry: bool,
    pub detail_level: i64,
    pub graphics_profile: &'static str,
    pub summaries: Vec<Summary>,
    pub elements: Vec<SavedElement>,
}
fn saved_element_status(meshes: &GraphicsMeshes) -> &'static str {
    // Selection exclusions are intentional profile decisions and stay visible
    // as counters without tainting an otherwise valid mesh. An unbounded face,
    // or a decoder diagnostic, means the owner was not fully tessellated.
    if meshes.diagnostics.is_empty() && meshes.unbounded_faces == 0 {
        "decoded_saved_graphics"
    } else {
        "partial_saved_graphics"
    }
}
/// The saved-material resolver alone (the channel-102 pass of
/// [`extract_at_detail`]), for callers that look materials up by id.
pub fn material_resolver(
    file: &mut RevitFile,
    options: &Options,
) -> Result<crate::native_saved_materials::Resolver> {
    let mut resolver = crate::native_saved_materials::Resolver::default_viewport_profile();
    let mut opts = options.clone();
    opts.channels = BTreeSet::from([102]);
    opts.selected_ids.clear();
    // the saved graphics and materials read on 2015 to 2022 too.
    opts.releases = native_document::FRAMED_RELEASES;
    native_document::extract(file, &opts, |r| resolver.ingest(&r))?;
    Ok(resolver)
}
pub fn extract(file: &mut RevitFile, options: &Options) -> Result<SavedScene> {
    extract_at_detail(file, options, 3)
}
/// [`extract`], also decoding the branches whose visibility flags hide them;
/// each primitive one lies on the way to is marked hidden.
pub fn extract_including_hidden(file: &mut RevitFile, options: &Options) -> Result<SavedScene> {
    extract_impl(
        file,
        options,
        3,
        &WalkerControl::default(),
        false,
        None,
        true,
        false,
    )
}
/// [`extract_including_hidden`], also decoding the children of a filter whose
/// conditions reject the context or are not read; each primitive one lies on
/// the way to is marked filtered.
pub fn extract_including_filtered(file: &mut RevitFile, options: &Options) -> Result<SavedScene> {
    extract_impl(
        file,
        options,
        3,
        &WalkerControl::default(),
        false,
        None,
        true,
        true,
    )
}
pub fn extract_at_detail(
    file: &mut RevitFile,
    options: &Options,
    detail_level: i64,
) -> Result<SavedScene> {
    extract_at_detail_with_control(file, options, detail_level, &WalkerControl::default())
}
/// Same as [`extract_at_detail`], reporting [`Stage::SavedGraphics`] and
/// [`Stage::SavedMeshes`] through `control` and stopping with
/// [`crate::Error::Cancelled`] when its token is cancelled.
pub fn extract_at_detail_with_control(
    file: &mut RevitFile,
    options: &Options,
    detail_level: i64,
    control: &WalkerControl,
) -> Result<SavedScene> {
    extract_framed_with_control(file, options, detail_level, control, false)
}
/// [`extract_at_detail_with_control`], with `keep_frame` leaving
/// each primitive in its source graph's frame (`Primitive::frame`).
pub fn extract_framed_with_control(
    file: &mut RevitFile,
    options: &Options,
    detail_level: i64,
    control: &WalkerControl,
    keep_frame: bool,
) -> Result<SavedScene> {
    extract_framed_with_materials(file, options, detail_level, control, keep_frame, None)
}
/// The saved-material resolver [`extract_framed_with_control`] builds: one
/// pass over every element's channel-102 records, whatever
/// `options.selected_ids` holds. A caller extracting the saved graphics in
/// batches builds it once and hands it to [`extract_framed_with_materials`].
pub fn saved_material_resolver(
    file: &mut RevitFile,
    options: &Options,
) -> Result<crate::native_saved_materials::Resolver> {
    let mut resolver = crate::native_saved_materials::Resolver::default_viewport_profile();
    native_document::extract_graphs(file, &material_options(options), |r| resolver.ingest(&r))?;
    Ok(resolver)
}
fn material_options(options: &Options) -> Options {
    let mut material_options = options.clone();
    material_options.channels = BTreeSet::from([102]);
    material_options.selected_ids.clear();
    material_options.releases = native_document::FRAMED_RELEASES;
    material_options
}
/// [`extract_framed_with_control`], resolving materials with `materials`
/// (from [`saved_material_resolver`] with the same options apart from
/// `selected_ids`) instead of building the resolver again. The scene is the
/// same, except that `summaries` then leaves out the material pass's summary.
pub fn extract_framed_with_materials(
    file: &mut RevitFile,
    options: &Options,
    detail_level: i64,
    control: &WalkerControl,
    keep_frame: bool,
    materials: Option<&crate::native_saved_materials::Resolver>,
) -> Result<SavedScene> {
    extract_impl(
        file,
        options,
        detail_level,
        control,
        keep_frame,
        materials,
        false,
        false,
    )
}
#[allow(clippy::too_many_arguments)]
fn extract_impl(
    file: &mut RevitFile,
    options: &Options,
    detail_level: i64,
    control: &WalkerControl,
    keep_frame: bool,
    materials: Option<&crate::native_saved_materials::Resolver>,
    include_hidden: bool,
    include_filtered: bool,
) -> Result<SavedScene> {
    ensure!(
        (1..=3).contains(&detail_level),
        "detail level must be 1, 2 or 3"
    );
    control.check()?;
    control.report(Stage::SavedGraphics, 0, None);
    let mut opts = options.clone();
    opts.channels = BTreeSet::from([103]);
    // Revit 2014 to 2022 frame their saved graphics as 2023 does
    // (rac_basic 2015-2022: the same 324 whole meshes each).
    opts.releases = native_document::FRAMED_RELEASES;
    // Each element's `source` below carries its record's body digest.
    opts.record_extras = true;
    let mut records = BTreeMap::<u64, Record>::new();
    let first = native_document::extract_graphs(file, &opts, |r| {
        ensure!(
            records.insert(r.identity.element_id, r).is_none(),
            "duplicate current graphics owner"
        );
        Ok(())
    })?;
    let roots: BTreeSet<_> = records.keys().copied().collect();
    let mut summaries = vec![first];
    let mut attempted = roots.clone();
    for depth in 0..=128 {
        let mut wanted = BTreeSet::new();
        for r in records.values() {
            if let Some(g) = &r.graph {
                for o in &g.objects {
                    if o.class_name == "InstanceInfo" {
                        if let Some(id) =
                            crate::native_metadata::identifier(&o.fields["m_symbolId"])
                                .ok()
                                .and_then(|id| u64::try_from(id).ok())
                                .filter(|id| *id > 0)
                        {
                            if !attempted.contains(&id) {
                                wanted.insert(id);
                            }
                        }
                    }
                }
            }
        }
        if wanted.is_empty() {
            break;
        }
        ensure!(depth < 128, "shared graphics closure depth budget");
        control.check()?;
        attempted.extend(&wanted);
        opts.selected_ids = wanted;
        summaries.push(native_document::extract_graphs(file, &opts, |r| {
            ensure!(
                records.insert(r.identity.element_id, r).is_none(),
                "duplicate shared graphics owner"
            );
            Ok(())
        })?);
    }
    control.check()?;
    let built;
    let resolver = match materials {
        Some(resolver) => resolver,
        None => {
            let mut resolver = crate::native_saved_materials::Resolver::default_viewport_profile();
            summaries.push(native_document::extract_graphs(
                file,
                &material_options(options),
                |r| resolver.ingest(&r),
            )?);
            built = resolver;
            &built
        }
    };
    control.report(Stage::SavedGraphics, 1, Some(1));
    // Elements mesh independently: spread them over the cores, keeping order.
    // The browser build (wasm32- and wasm64-unknown-unknown) has no threads, so there, as
    // on a single core, they mesh one after the other.
    let roots: Vec<u64> = roots.into_iter().collect();
    let total = roots.len() as u64;
    control.report(Stage::SavedMeshes, 0, Some(total));
    let mesh = |id: u64| -> Result<SavedElement> {
        let r = &records[&id];
        let mut meshes = match &r.graph {
            Some(g) if include_filtered => native_saved_mesh::graphics_including_filtered(
                g,
                &|id| records.get(&id).and_then(|r| r.graph.as_ref()),
                detail_level,
            ),
            Some(g) if include_hidden => native_saved_mesh::graphics_including_hidden(
                g,
                &|id| records.get(&id).and_then(|r| r.graph.as_ref()),
                detail_level,
            ),
            Some(g) => native_saved_mesh::graphics_framed(
                g,
                &|id| records.get(&id).and_then(|r| r.graph.as_ref()),
                detail_level,
                keep_frame,
            ),
            None => GraphicsMeshes::default(),
        };
        if r.graph.is_none() {
            meshes.diagnostics.push(
                r.diagnostic
                    .clone()
                    .unwrap_or_else(|| "graphics graph unavailable".into()),
            )
        }
        let refs: BTreeSet<_> = meshes
            .primitives
            .iter()
            .filter_map(|p| p.source_owner_id)
            .collect();
        let referenced_graph_sources = refs
            .into_iter()
            .map(|id| Ok((id, serde_json::to_value(&records[&id].source)?)))
            .collect::<Result<_>>()?;
        let mut render_materials = BTreeMap::new();
        let mut unresolved_material_primitives = Vec::new();
        for (index, p) in meshes.primitives.iter().enumerate() {
            match resolver.resolve(
                id,
                p.source_owner_id,
                p.face_tag,
                p.render_style_id,
                p.material_id,
            ) {
                Some(m) => {
                    render_materials.insert(index, m);
                }
                None => unresolved_material_primitives.push(index),
            }
        }
        Ok(SavedElement {
            render_materials,
            unresolved_material_primitives,
            id,
            identity: r.identity.clone(),
            source: serde_json::to_value(&r.source)?,
            referenced_graph_sources,
            status: saved_element_status(&meshes).into(),
            meshes,
        })
    };
    // Counted under a lock so the reports stay in order across workers.
    let done = std::sync::Mutex::new(0u64);
    let build = |id: u64| -> Result<SavedElement> {
        control.check()?;
        let element = mesh(id);
        let mut done = done.lock().unwrap_or_else(|e| e.into_inner());
        *done += 1;
        control.report(Stage::SavedMeshes, *done, Some(total));
        element
    };
    let workers = if cfg!(all(target_family = "wasm", target_os = "unknown")) {
        1
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    };
    let elements = if workers == 1 {
        roots
            .iter()
            .map(|&id| build(id))
            .collect::<Result<Vec<_>>>()?
    } else {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let mut built: Vec<(usize, Result<SavedElement>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    scope.spawn(|| {
                        let mut out = Vec::new();
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(&id) = roots.get(i) else { break };
                            out.push((i, build(id)));
                        }
                        out
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                .collect()
        });
        built.sort_by_key(|(i, _)| *i);
        built
            .into_iter()
            .map(|(_, e)| e)
            .collect::<Result<Vec<_>>>()?
    };
    // Freeing the decoded graphs takes seconds on large models: do it aside
    // where there are threads.
    if workers > 1 {
        std::thread::spawn(move || drop(records));
    }
    Ok(SavedScene {
        source_sha256: None,
        format: "rvt-native-saved-scene-v1",
        units: "feet",
        coordinate_frame: "document Z-up",
        complete_document_geometry: false,
        detail_level,
        graphics_profile: "default 3D viewport (qualified supported subset)",
        summaries,
        elements,
    })
}
