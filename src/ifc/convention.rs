//!
//! `revit` writes what Revit's own IFC export writes; `open` writes what the
//! model means. Every place the two differ is a method here, matched on
//! `self` once, with the evidence beside it. No other code branches on the
//! convention.

use super::Storey;
use crate::level_bind::LevelStoreyBind;

/// Which convention an export follows.
///
/// `Default` is `Open`: a model built by hand (a test fixture, the
/// The exporters and `rvt-ifc` default to [`Self::EXPORT_DEFAULT`], `Revit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportConvention {
    /// Revit's own IFC export: classes, containment and sets as it writes them.
    Revit,
    /// rvt-open's own: what the model means, where Revit's export bends it.
    #[default]
    Open,
}

impl ExportConvention {
    pub const EXPORT_DEFAULT: Self = Self::Revit;

    /// Switch point 1: the storey of a beam that no placing rule placed (no
    /// beam system, no Reference Level at or below a storey, no Level).
    ///
    /// - `Revit`: the storey of the Level with the lowest ElementId. Revit's
    ///   export did this for 8 of 8 such elements (6 DACH IfcMember with a
    ///   Reference Level below every storey, whose lowest-elevation storey is
    /// - `Open`: the lowest building storey by elevation.
    pub(crate) fn no_rule_beam_storey(
        self,
        bind: &LevelStoreyBind,
        storeys: &[Storey],
    ) -> (Option<usize>, &'static str) {
        match self {
            Self::Revit => (
                bind.lowest_id_storey_index(),
                crate::beam_reference_levels::STOREY_BIND_LOWEST_ID_STOREY_FALLBACK,
            ),
            Self::Open => (
                storeys
                    .iter()
                    .enumerate()
                    .min_by(|a, b| a.1.elevation_feet.total_cmp(&b.1.elevation_feet))
                    .map(|(index, _)| index),
                crate::beam_reference_levels::STOREY_BIND_LOWEST_ELEVATION_FALLBACK,
            ),
        }
    }

    /// Switch point 3: the IFC class of a Structural Framing instance.
    ///
    /// - `Revit`: by the element's structural type, as Revit's export does:
    ///   Beam is `IfcBeam` `.BEAM.` (2,338 of 2,338), Brace is `IfcMember`
    ///   `.BRACE.` (81 of 81), NonStructural is `IfcBuildingElementProxy`
    ///   `.NOTDEFINED.` (2 of 2); 2,421 of 2,421 on the 26 Revit 2024 samples
    ///   with a Revit IFC (`partition_framing_types`). The
    ///   structural type is read on 2011 to 2027.
    /// - `Open`: Brace is `IfcMember` `.BRACE.`, the
    ///   model's own beam/brace statement; Beam and NonStructural stay
    ///   `IfcBeam`, since a proxy would lose that the element is framing.
    pub(crate) fn framing_class(
        self,
        layout: crate::partition_framing_types::FramingLayout,
    ) -> (&'static str, &'static str) {
        use crate::partition_framing_types::FramingLayout;
        match (self, layout) {
            (_, FramingLayout::Brace) => ("IFCMEMBER", "BRACE"),
            (Self::Revit, FramingLayout::Other) => ("IFCBUILDINGELEMENTPROXY", "NOTDEFINED"),
            _ => ("IFCBEAM", "BEAM"),
        }
    }

    /// Switch point 4: design options.
    ///
    /// - `Revit`: the main model and each set's primary option only, as
    ///   Revit's export writes them; no design option groups.
    /// - `Open`: the elements of every option, each assigned to an
    ///   `IfcGroup` of its option inside an `IfcGroup` of its set, and
    ///   carrying a `Design Option` property set (set, option, primary).
    pub fn writes_every_design_option(self) -> bool {
        match self {
            Self::Revit => false,
            Self::Open => true,
        }
    }

    /// Switch point 2: a model with no building storey.
    ///
    /// - `Revit`: no `IfcBuilding` and no storey at all; the `IfcSite` itself
    ///   contains every element that is not part of an aggregate and
    ///   aggregates the spaces. Measured on rstbasicsampleproject:
    ///   3,949 elements in the one `IfcSite`, 0 `IfcBuilding`, 0 storeys, the
    ///   one `IfcSpace` aggregated by the site, nothing uncontained.
    /// - `Open`: the `IfcBuilding` "Default Building" stays and holds them.
    ///
    /// True when the site is the root that holds the elements.
    pub fn site_holds_elements(self, no_storeys: bool) -> bool {
        match self {
            Self::Revit => no_storeys,
            Self::Open => false,
        }
    }

    /// Switch point 2, second half: whether an element-record elevation join
    /// may invent storeys when the file's Levels gave none. Revit writes no
    /// storey for such a model (rstbasicsampleproject: 0), so under `Revit`
    /// the join does not invent any.
    pub(crate) fn invents_storeys_from_elevations(self) -> bool {
        match self {
            Self::Revit => false,
            Self::Open => true,
        }
    }
    /// Switch point 5: an `IfcSpace`'s predefined type where its
    /// "IFC Predefined Type" parameter is not set. `kind` is the Revit
    /// spatial element: `Room`, `Area` (area scheme) or `Space` (MEP).
    ///
    /// - `Revit`: `NOTDEFINED`, as Revit 2027.1's export of the 12 Nordic LCA
    ///   models writes 499 of 499 such rooms and 200 of 200 areas.
    /// - `Open`: a room or MEP space is `SPACE`, the predefined type that says
    ///   it is an ordinary space rather than leaving it unknown (thread
    ///   `ifc-spaces-zones-boundaries-for-analysis`); an area stays
    ///   `NOTDEFINED`, except an area of a Gross Building area scheme
    ///   (`gross`), which is a `GFA` space (48 of the
    ///   Nordic LCA's 200 areas).
    pub(crate) fn space_predefined_type(self, kind: &str, gross: bool) -> &'static str {
        match (self, kind) {
            (Self::Open, "Room" | "Space") => "SPACE",
            (Self::Open, "Area") if gross => "GFA",
            _ => "NOTDEFINED",
        }
    }
    /// Switch point 6: where a family instance exported as
    /// `IfcSpace` ("Export to IFC As") goes.
    ///
    /// - `Revit`: contained in its storey like any element, as Revit 2027.1's
    ///   export writes 352 of 352 on the Nordic LCA models.
    /// - `Open`: aggregated by its storey, as every space is: a space is part
    ///   of the spatial breakdown, not an element in it.
    pub fn contains_family_spaces(self) -> bool {
        match self {
            Self::Revit => true,
            Self::Open => false,
        }
    }
    /// Switch point 7: HVAC zones.
    ///
    /// - `Revit`: none. Revit's exporter (read for its rules,
    ///   `ZoneName` parameters or a Revit 2026+ generic zone, never from an
    ///   HVAC zone; Revit 2027.1's export of the 12 Nordic LCA models has
    ///   no `IfcZone`.
    /// - `Open`: an `IfcZone` per HVAC zone with written spaces, grouping
    ///   them (the zone a space is in is part of the model).
    pub fn writes_hvac_zones(self) -> bool {
        match self {
            Self::Revit => false,
            Self::Open => true,
        }
    }
}
