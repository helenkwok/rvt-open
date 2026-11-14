//! Every tests/*.rs file as a module of one test binary: 77 binaries
//! took 118 s of build time where this one takes 13 s. A file's tests are
//! named `<file>::<test>`; tools/ci/verify-real-files.sh fails when a file
//! has no line here.

// Each file declares its own `mod common;`, so tests/common is compiled once
// per file that uses it, as when every file was its own binary.
#![allow(clippy::duplicate_mod)]

/// one test per shard of a test over several sample files, so
/// nextest runs them side by side. `split!(name, body, s0 0, s1 1, ...)` makes
/// the tests `name::s0`, `name::s1`, ... that call `body(k, n)`, `n` being the
/// number of shards; `body` checks [`shard`]`(items, k, n)`, so the shards
/// together check every item the one test did.
macro_rules! split {
    ($name:ident, $body:ident, $($s:ident $k:literal),+ $(,)?) => {
        mod $name {
            const N: usize = [$($k),+].len();
            // The shards are numbered 0, 1, ..., N - 1, so none is left out.
            const _: () = {
                let ks: [usize; N] = [$($k),+];
                let mut i = 0;
                while i < N {
                    assert!(ks[i] == i);
                    i += 1;
                }
            };
            $(
                #[test]
                fn $s() {
                    super::$body($k, N)
                }
            )+
        }
    };
}

/// Shard `k` of `n`: every `n`-th item from item `k`. The `n` shards hold every
/// item once, however many items there are.
pub(crate) fn shard<T>(items: impl IntoIterator<Item = T>, k: usize, n: usize) -> Vec<T> {
    items
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % n == k)
        .map(|(_, item)| item)
        .collect()
}

/// A check over all shards of a split test (a total): records `value` as shard
/// `k`'s and returns every shard's value, by shard, to the shard whose record
/// completes the set, and `None` to the others. Each shard writes before it
/// reads, so the last to write sees all `n`. Records live under
/// `$CARGO_TARGET_TMPDIR/split-parts/<run>/<check>`, `<run>` being nextest's run
/// id (each test is its own process) or, under libtest, this process.
pub(crate) fn all_shards(check: &str, k: usize, n: usize, value: &str) -> Option<Vec<String>> {
    let run = std::env::var("NEXTEST_RUN_ID").unwrap_or_else(|_| std::process::id().to_string());
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("split-parts")
        .join(run)
        .join(check);
    std::fs::create_dir_all(&dir).expect("split-parts dir");
    let tmp = dir.join(format!("{k}.tmp"));
    std::fs::write(&tmp, value).expect("write shard record");
    std::fs::rename(&tmp, dir.join(k.to_string())).expect("publish shard record");
    (0..n)
        .map(|i| std::fs::read_to_string(dir.join(i.to_string())).ok())
        .collect()
}

#[path = "../arc_wall_corpus.rs"]
mod arc_wall_corpus;
#[path = "../assoc_levels_2022.rs"]
mod assoc_levels_2022;
#[path = "../assoc_levels_2027.rs"]
mod assoc_levels_2027;
#[path = "../beam_reference_levels.rs"]
mod beam_reference_levels;
#[path = "../beam_storeys.rs"]
mod beam_storeys;
#[path = "../beam_system_params.rs"]
mod beam_system_params;
#[path = "../binary_inventory.rs"]
mod binary_inventory;
#[path = "../boxless_room.rs"]
mod boxless_room;
#[path = "../category_type_phase.rs"]
mod category_type_phase;
#[path = "../cli_ergonomics.rs"]
mod cli_ergonomics;
#[path = "../cli_real_files.rs"]
mod cli_real_files;
#[path = "../column_cut_body.rs"]
mod column_cut_body;
#[path = "../column_node_z.rs"]
mod column_node_z;
#[path = "../common_reference.rs"]
mod common_reference;
#[path = "../constraint_levels.rs"]
mod constraint_levels;
#[path = "../corpus_tier2_health.rs"]
mod corpus_tier2_health;
#[path = "../covering_finish.rs"]
mod covering_finish;
#[path = "../curtain_part_materials.rs"]
mod curtain_part_materials;
#[path = "../curtain_wall_doors.rs"]
mod curtain_wall_doors;
#[path = "../curtain_walls.rs"]
mod curtain_walls;
#[path = "../data_outputs.rs"]
mod data_outputs;
#[path = "../decoder_diffs.rs"]
mod decoder_diffs;
#[path = "../design_options.rs"]
mod design_options;
#[path = "../design_options_open.rs"]
mod design_options_open;
#[path = "../door_overall_sizes.rs"]
mod door_overall_sizes;
#[path = "../door_type_global_ids.rs"]
mod door_type_global_ids;
#[path = "../door_window_wall_negative.rs"]
mod door_window_wall_negative;
#[path = "../duct_and_pipe_ports.rs"]
mod duct_and_pipe_ports;
#[path = "../duct_fields.rs"]
mod duct_fields;
#[path = "../duct_names.rs"]
mod duct_names;
#[path = "../duct_segment_shape.rs"]
mod duct_segment_shape;
#[path = "../duct_segment_type_common.rs"]
mod duct_segment_type_common;
#[path = "../elem_table_corpus.rs"]
mod elem_table_corpus;
#[path = "../elem_table_frame.rs"]
mod elem_table_frame;
#[path = "../element_materials.rs"]
mod element_materials;
#[path = "../element_names.rs"]
mod element_names;
#[path = "../element_parameters.rs"]
mod element_parameters;
#[path = "../element_provenance_offsets.rs"]
mod element_provenance_offsets;
#[path = "../element_record_stages.rs"]
mod element_record_stages;
#[path = "../element_records_2025.rs"]
mod element_records_2025;
#[path = "../export_as_2018_2023.rs"]
mod export_as_2018_2023;
#[path = "../export_conventions.rs"]
mod export_conventions;
#[path = "../export_decisions_counts.rs"]
mod export_decisions_counts;
#[path = "../family_blend_transitions.rs"]
mod family_blend_transitions;
#[path = "../family_instance_ports.rs"]
mod family_instance_ports;
#[path = "../field_type_coverage.rs"]
mod field_type_coverage;
#[path = "../fitting_nominal_diameter.rs"]
mod fitting_nominal_diameter;
#[path = "../floor_outline_mesh.rs"]
mod floor_outline_mesh;
#[path = "../floor_plane_joint.rs"]
mod floor_plane_joint;
#[path = "../frag_writer.rs"]
mod frag_writer;
#[path = "../fuzz_regressions.rs"]
mod fuzz_regressions;
#[path = "../geometry_invariants.rs"]
mod geometry_invariants;
#[path = "../georeference.rs"]
mod georeference;
#[path = "../gltf_normals.rs"]
mod gltf_normals;
#[path = "../guid_namespace.rs"]
mod guid_namespace;
#[path = "../ifc_export_overrides.rs"]
mod ifc_export_overrides;
#[path = "../ifc_roundtrip.rs"]
mod ifc_roundtrip;
#[path = "../ifc_synthetic_project.rs"]
mod ifc_synthetic_project;
#[path = "../ifc_synthetic_structural.rs"]
mod ifc_synthetic_structural;
#[path = "../ifcx_writer.rs"]
mod ifcx_writer;
#[path = "../invert_elevation.rs"]
mod invert_elevation;
#[path = "../is_external.rs"]
mod is_external;
#[path = "../iter_elements_typed.rs"]
mod iter_elements_typed;
#[path = "../json_schema_contracts.rs"]
mod json_schema_contracts;
#[path = "../large_partition_file.rs"]
mod large_partition_file;
#[path = "../level_names.rs"]
mod level_names;
#[path = "../level_parameters.rs"]
mod level_parameters;
#[path = "../level_shared_elevation.rs"]
mod level_shared_elevation;
#[path = "../memory_staging.rs"]
mod memory_staging;
#[path = "../mep_categories.rs"]
mod mep_categories;
#[path = "../mep_connections.rs"]
mod mep_connections;
#[path = "../mep_curtain_type_links.rs"]
mod mep_curtain_type_links;
#[path = "../mep_curve_types.rs"]
mod mep_curve_types;
#[path = "../mep_phase_created.rs"]
mod mep_phase_created;
#[path = "../mep_systems.rs"]
mod mep_systems;
#[path = "../native_current_records.rs"]
mod native_current_records;
#[path = "../native_partitions_walk.rs"]
mod native_partitions_walk;
#[path = "../native_phases.rs"]
mod native_phases;
#[path = "../parameter_diagnostics.rs"]
mod parameter_diagnostics;
#[path = "../partition_record_chain.rs"]
mod partition_record_chain;
#[path = "../partition_scanner.rs"]
mod partition_scanner;
#[path = "../pdf3d_export.rs"]
mod pdf3d_export;
#[path = "../phase_filter.rs"]
mod phase_filter;
#[path = "../plan_maps_and_spaces.rs"]
mod plan_maps_and_spaces;
#[path = "../port_nesting.rs"]
mod port_nesting;
#[path = "../project_corpus_smoke.rs"]
mod project_corpus_smoke;
#[path = "../project_count_fixtures.rs"]
mod project_count_fixtures;
#[path = "../proxy_ports.rs"]
mod proxy_ports;
#[path = "../railing_type.rs"]
mod railing_type;
#[path = "../reference_values.rs"]
mod reference_values;
#[path = "../regression_fixes.rs"]
mod regression_fixes;
#[path = "../release_coverage.rs"]
mod release_coverage;
#[path = "../reproducible_outputs.rs"]
mod reproducible_outputs;
#[path = "../resave_stable_ids.rs"]
mod resave_stable_ids;
#[path = "../resource_limits_cli.rs"]
mod resource_limits_cli;
#[path = "../revit_2016.rs"]
mod revit_2016;
#[path = "../revit_global_ids.rs"]
mod revit_global_ids;
#[path = "../revit_maps_and_types.rs"]
mod revit_maps_and_types;
#[path = "../revit_property_sets.rs"]
mod revit_property_sets;
#[path = "../roof_butterfly.rs"]
mod roof_butterfly;
#[path = "../roof_extrusion_holes.rs"]
mod roof_extrusion_holes;
#[path = "../rooms_floors_from_records_only.rs"]
mod rooms_floors_from_records_only;
#[path = "../rvt_dump_cli.rs"]
mod rvt_dump_cli;
#[path = "../rvt_gltf_progress_cli.rs"]
mod rvt_gltf_progress_cli;
#[path = "../rvt_ifc_diagnostics_cli.rs"]
mod rvt_ifc_diagnostics_cli;
#[path = "../rvt_info_cli.rs"]
mod rvt_info_cli;
#[path = "../rvt_inspect_cli.rs"]
mod rvt_inspect_cli;
#[path = "../rvt_open_hints.rs"]
mod rvt_open_hints;
#[path = "../rvt_schedule_cli.rs"]
mod rvt_schedule_cli;
#[path = "../samples.rs"]
mod samples;
#[path = "../saved_mesh_budget.rs"]
mod saved_mesh_budget;
#[path = "../scan_limits.rs"]
mod scan_limits;
#[path = "../schema_bbox_marker.rs"]
mod schema_bbox_marker;
#[path = "../schema_objects.rs"]
mod schema_objects;
#[path = "../schema_registry_catalogs.rs"]
mod schema_registry_catalogs;
#[path = "../segment_length.rs"]
mod segment_length;
#[path = "../serial_number.rs"]
mod serial_number;
#[path = "../shaft_sketch_loops.rs"]
mod shaft_sketch_loops;
#[path = "../shared_shapes.rs"]
mod shared_shapes;
#[path = "../sheet_view.rs"]
mod sheet_view;
#[path = "../skipped_members.rs"]
mod skipped_members;
#[path = "../space_common.rs"]
mod space_common;
#[path = "../space_containment.rs"]
mod space_containment;
#[path = "../space_types.rs"]
mod space_types;
#[path = "../spatial_property_sets.rs"]
mod spatial_property_sets;
#[path = "../stable_exports.rs"]
mod stable_exports;
#[path = "../stream_size_guards.rs"]
mod stream_size_guards;
#[path = "../streamed_partition.rs"]
mod streamed_partition;
#[path = "../support_matrix.rs"]
mod support_matrix;
#[path = "../tapered_walls_2025.rs"]
mod tapered_walls_2025;
#[path = "../truss_framing.rs"]
mod truss_framing;
#[path = "../type_global_ids.rs"]
mod type_global_ids;
#[path = "../typed_no_geometry_types.rs"]
mod typed_no_geometry_types;
#[path = "../unfilled_openings.rs"]
mod unfilled_openings;
#[path = "../untyped_elements.rs"]
mod untyped_elements;
#[path = "../usd_writer.rs"]
mod usd_writer;
#[path = "../value_sources_offsets.rs"]
mod value_sources_offsets;
#[path = "../walker_to_ifc_integration.rs"]
mod walker_to_ifc_integration;
#[path = "../wall_layer_joins.rs"]
mod wall_layer_joins;
#[path = "../wall_profile_openings.rs"]
mod wall_profile_openings;
#[path = "../witness_registry.rs"]
mod witness_registry;
#[path = "../witness_verdict.rs"]
mod witness_verdict;
