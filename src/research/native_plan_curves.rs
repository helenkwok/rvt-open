//! A family symbol's plan-view curves from its saved graphics.
//!
//! Revit's IFC export writes a family type's `FootPrint` map from the curves
//! the symbol's geometry yields in the instance level's plan view: the free
//! curves at the symbol's own top level (symbolic lines and model lines a
//! plan view shows), never those of nested families. The saved graphics graph
//! of the symbol holds those curves under `GFilter` nodes whose conditions
//! name a view direction, a view type and a detail level; this walks the graph
//! as a plan view sees it (direction +Z, view type 1).
use crate::graph_value::Fv;
use crate::native_graphics_traversal::{
    GraphicsTransform, condition_in_view, direction_condition_for, graphics_visibility,
};
use crate::native_parameters::ObjectGraph;
use crate::native_parametric_surface::Curve;
use std::collections::BTreeMap;

const IDENTITY: GraphicsTransform = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];
/// A plan view's direction, toward the viewer.
pub const PLAN_VIEW: [f64; 3] = [0., 0., 1.];

/// How a plan view reads the saved conditions.
#[derive(Debug, Clone, Copy)]
pub struct PlanView {
    /// 1 coarse, 2 medium, 3 fine.
    pub detail_level: i64,
    /// Which graphics visibility codes (0..=3) the view draws; the default 3D
    /// view draws 0 and 2.
    pub visible: [bool; 4],
    /// Direction predicates 6, 7 and 8 (false in the default 3D view), each
    /// read as: 0 false, 1 the condition's direction parallel to the view's,
    /// 2 the same direction, 3 the opposite one.
    pub predicates: [u8; 3],
    /// `GConditionCut`: evaluated like a direction predicate (`true`), or
    /// false before negation (`false`: the symbol is not cut).
    pub cut_as_direction: bool,
}

/// The view measured against Revit 2027.1's own `FootPrint` maps on the 12
/// Nordic LCA models: of 545 family types with one, the curves
/// read cover the same plan extent within 1 cm on 517 and have Revit's curve
/// count on 489 (529 have curves) with visibility code 1 not drawn, no cut,
/// medium detail, and the direction predicates read as 6 parallel to the
/// view, 7 the opposite direction (a reflected ceiling plan's) and 8 the
/// same direction. Of the 64 readings of 6/7/8 tried, none does better; 7
/// read as true for a plan view (as an earlier reading had it) doubles
/// curves and leaves 17 types at Revit's count.
impl Default for PlanView {
    fn default() -> Self {
        Self {
            detail_level: 2,
            visible: [true, false, true, false],
            predicates: [1, 3, 2],
            cut_as_direction: false,
        }
    }
}

/// One curve a plan view shows, in the symbol's coordinates (feet).
#[derive(Debug, Clone)]
pub struct PlanCurve {
    pub curve: Curve,
    pub transform: GraphicsTransform,
}

/// What [`plan_curves`] found.
#[derive(Debug, Clone, Default)]
pub struct PlanCurves {
    pub curves: Vec<PlanCurve>,
    /// Curve classes not decoded yet (ellipses, splines, ...), by class.
    pub unsupported: BTreeMap<String, usize>,
    /// Branches a condition hid from the plan view.
    pub hidden: usize,
    /// The condition that hid each hidden branch, by its class and fields.
    pub hidden_by: BTreeMap<String, usize>,
    /// Branches whose condition could not be evaluated, by message.
    pub unknown: BTreeMap<String, usize>,
    /// Nested family instances (their curves are not the symbol's own).
    pub nested: usize,
}

/// The plan-view curves at the top level of a symbol's saved graphics.
pub fn plan_curves(graph: &ObjectGraph, view: &PlanView) -> PlanCurves {
    let mut out = PlanCurves::default();
    if !graph.objects.is_empty() {
        visit(graph, 0, IDENTITY, view, &mut out, 0);
    }
    out
}

fn alignment(v: &Fv, m: GraphicsTransform) -> Result<f64, String> {
    let a = v["m_dir"]
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or("missing filter direction")?;
    let local: Vec<f64> = a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect();
    let world: Vec<f64> = (0..3)
        .map(|i| (0..3).map(|j| m[i][j] * local[j]).sum())
        .collect();
    let len = world.iter().map(|x| x * x).sum::<f64>().sqrt();
    if len.is_nan() || len <= 1e-12 {
        return Err("zero filter direction".into());
    }
    let dot: f64 = (0..3).map(|i| world[i] * PLAN_VIEW[i]).sum::<f64>() / len;
    Ok(dot)
}

fn children(g: &ObjectGraph, index: usize) -> Option<Vec<usize>> {
    g.objects[index].fields["m_subNodes"]
        .as_array()?
        .iter()
        .map(|p| {
            crate::native_saved_mesh::pointer(g, index, p)
                .ok()
                .flatten()
        })
        .collect()
}

fn condition(
    g: &ObjectGraph,
    ci: usize,
    m: GraphicsTransform,
    view: &PlanView,
) -> Result<bool, String> {
    let o = &g.objects[ci];
    let f = &o.fields;
    match o.class_name.as_str() {
        "GConditionDir" | "GConditionCut" => {
            let comp = f["m_comp"].as_i64().ok_or("missing direction comparison")?;
            let negate = f["m_negateDirCondition"]
                .as_bool()
                .ok_or("missing direction negation")?;
            if o.class_name == "GConditionCut" && !view.cut_as_direction {
                return Ok(negate);
            }
            if matches!(comp, 6..=8) {
                let dot = alignment(f, m)?;
                let answer = match view.predicates[(comp - 6) as usize] {
                    1 => dot.abs() > 1. - 1e-9,
                    2 => dot > 1. - 1e-9,
                    3 => dot < -1. + 1e-9,
                    _ => false,
                };
                return Ok(answer != negate);
            }
            direction_condition_for(f, m, PLAN_VIEW)
        }
        _ => condition_in_view(&o.class_name, f, view.detail_level, 1),
    }
}

fn visit(
    g: &ObjectGraph,
    index: usize,
    m: GraphicsTransform,
    view: &PlanView,
    out: &mut PlanCurves,
    depth: usize,
) {
    if depth > 128 {
        *out.unknown.entry("depth".into()).or_default() += 1;
        return;
    }
    let obj = &g.objects[index];
    if let Some(flags) = obj.fields["m_GInfo"]["m_flags"].as_u64() {
        if !view.visible[graphics_visibility(flags) as usize] {
            out.hidden += 1;
            *out.hidden_by
                .entry(format!("visibility flags {flags:#x}"))
                .or_default() += 1;
            return;
        }
    }
    match obj.class_name.as_str() {
        "GElement" | "GGroup" | "GFilter" => {
            if obj.class_name == "GFilter" {
                let conditions: &[Fv] = obj.fields["m_oConditions"].as_array().map_or(&[], |v| v);
                for p in conditions {
                    let Some(ci) = crate::native_saved_mesh::pointer(g, index, p)
                        .ok()
                        .flatten()
                    else {
                        *out.unknown.entry("condition pointer".into()).or_default() += 1;
                        return;
                    };
                    match condition(g, ci, m, view) {
                        Ok(true) => {}
                        Ok(false) => {
                            out.hidden += 1;
                            let c = &g.objects[ci];
                            let f = &c.fields;
                            let sig = format!(
                                "{} comp={} param={} value={} dir={} neg={}",
                                c.class_name,
                                f["m_comp"],
                                f["m_param"],
                                f["m_value"],
                                f["m_dir"],
                                f["m_negateDirCondition"]
                            );
                            *out.hidden_by.entry(sig).or_default() += 1;
                            return;
                        }
                        Err(e) => {
                            *out.unknown.entry(e).or_default() += 1;
                            return;
                        }
                    }
                }
            }
            let Some(kids) = children(g, index) else {
                *out.unknown.entry("children".into()).or_default() += 1;
                return;
            };
            for k in kids {
                visit(g, k, m, view, out, depth + 1);
            }
        }
        "GInstance" | "GComponentRef" => out.nested += 1,
        "GLine" | "GArc" => match Curve::read(g, index) {
            Ok(curve) => out.curves.push(PlanCurve {
                curve,
                transform: m,
            }),
            Err(e) => *out.unknown.entry(e.to_string()).or_default() += 1,
        },
        "GEllipse" | "GCurve" | "GNurbSpline" | "GPolyLine" | "GPolyline" | "GPoint" => {
            *out.unsupported.entry(obj.class_name.clone()).or_default() += 1
        }
        _ => {}
    }
}
