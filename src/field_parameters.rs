//! Built-in parameters Revit reports from an element's own fields rather
//! than from its stored parameter sets.
//!
//! Most of what the Revit API lists as an element's parameters is not in its
//! `ParamValueSet*` objects ([`crate::schema_objects::SchemaObject::stored_parameters`]):
//! Revit shows a field of the element object, or of its element header, as
//! a built-in parameter. This module reads those.
//!
//! - From the element header ([`crate::schema_objects::header_records`]):
//!   `ELEM_CATEGORY_PARAM` and `ELEM_CATEGORY_PARAM_MT` are the header's
//!   category, `DESIGN_OPTION_ID` its design option (-4 read as -1, none).
//!   On 1-hip-roof (Revit 2024) these equal the API's value for 16,518 of
//!   16,536 parameters; the 18 others are the two category parameters of 9
//!   elements whose header holds no category or a parent category, where
//!   the API reports another (a subcategory).
//! - From the element object: one field per (schema class, parameter), the
//!   table below. Each rule was measured on 1-hip-roof against its Revit API
//!   dump: the field equals the API's value on every element of that class
//!   that has the parameter, and the field's name names the parameter or its
//!   values vary. 367 rules gave the API's value for 6,117 parameters;
//!   26 more follow a pointer into an object the element owns (a type's
//!   compound structure, a material's own object) and give 508 more. Other releases and files: not measured; a class without
//!   the field gives nothing.

use crate::builtin_parameters;
use crate::schema_objects::{Fields, ParamValue, Value};

/// (schema class of the element object, `BuiltInParameter` enum name, field
/// path: field names joined by `.` into objects held by value or pointed to).
pub const FIELD_RULES: &[(&str, &str, &str)] = &[
    (
        "AbsFlexDuctType",
        "RBS_CURVETYPE_DEFAULT_TAKEOFF_PARAM",
        "m_idDefaultTakeoff",
    ),
    (
        "AbsFlexDuctType",
        "RBS_CURVETYPE_DEFAULT_TEE_PARAM",
        "m_idDefaultTee",
    ),
    (
        "AbsFlexDuctType",
        "RBS_CURVETYPE_DEFAULT_TRANSITION_PARAM",
        "m_idDefaultTransition",
    ),
    (
        "AbsFlexDuctType",
        "RBS_CURVETYPE_DEFAULT_UNION_PARAM",
        "m_idDefaultUnion",
    ),
    ("AreaMeasureElem", "AREA_SCHEME_NAME", "m_areaElemName"),
    (
        "BasicWallType",
        "COARSE_SCALE_FILL_PATTERN_COLOR",
        "m_pCompoundStructure.m_coarseScaleFillColor",
    ),
    (
        "BasicWallType",
        "COARSE_SCALE_FILL_PATTERN_ID_PARAM",
        "m_pCompoundStructure.m_coarseScaleFillPatternElemId",
    ),
    ("BasicWallType", "FUNCTION_PARAM", "m_function"),
    ("CalloutTag", "CALLOUT_ATTR_HEAD_TAG", "m_familyTagId"),
    (
        "CalloutTag",
        "CALLOUT_CORNER_SHEET_RADIUS",
        "m_cornerRadius",
    ),
    (
        "ColorFillSymbol",
        "COLOR_FILL_SWATCH_HEIGHT_PARAM",
        "m_swatchHeight",
    ),
    (
        "ColorFillSymbol",
        "COLOR_FILL_SWATCH_WIDTH_PARAM",
        "m_swatchWidth",
    ),
    ("ColorFillSymbol", "SHOW_TITLE", "m_bShowTitle"),
    (
        "ConceptualConstructionType",
        "CONCEPTUAL_CONSTRUCTION_MATERIAL",
        "m_materialId",
    ),
    (
        "ContFootingType",
        "CONTINUOUS_FOOTING_BEARING_WIDTH",
        "m_bearingWidth",
    ),
    (
        "ContFootingType",
        "CONTINUOUS_FOOTING_BREAK_AT_INSERTS_DISABLE",
        "m_breakAtInsertsDisable",
    ),
    (
        "ContFootingType",
        "CONTINUOUS_FOOTING_DEFAULT_END_EXTENSION_LENGTH",
        "m_defaultEndExtLength",
    ),
    (
        "ContFootingType",
        "CONTINUOUS_FOOTING_STRUCTURAL_USAGE",
        "m_footingStructuralUsage",
    ),
    (
        "ContFootingType",
        "CONTINUOUS_FOOTING_TOP_HEEL",
        "m_topHeel",
    ),
    ("ContFootingType", "CONTINUOUS_FOOTING_TOP_TOE", "m_topToe"),
    (
        "ContourLabelingAttributes",
        "CONTOUR_LABELS_PRIMARY_ONLY",
        "m_makePrimaryOnly",
    ),
    ("CorniceAttr", "WALL_SWEEP_CUTS_WALL_PARAM", "m_cutsWall"),
    (
        "CorniceAttr",
        "WALL_SWEEP_CUT_BY_INSERTS_PARAM",
        "m_cutByInserts",
    ),
    (
        "CorniceAttr",
        "WALL_SWEEP_DEFAULT_SETBACK_PARAM",
        "m_defaultSetback",
    ),
    ("CurtaSystemAttr", "AUTO_JOIN_CONDITION", "m_autoJoinCond"),
    (
        "CurtainRoofAttributes",
        "AUTO_JOIN_CONDITION",
        "m_autoJoinCond",
    ),
    ("CutMarkType", "CUT_LINE_TYPE", "m_cutLineType"),
    ("CutMarkType", "CUT_MARK_SYMBOL", "m_cutMarkSymbol"),
    ("CutMarkType", "CUT_MARK_SYMBOL_SIZE", "m_cutMarkSymbolSize"),
    ("DBView3d", "VIEW_PARTS_VISIBILITY", "m_partsVisibility"),
    ("DBView3d", "VIEW_TEMPLATE_FOR_SCHEDULE", "m_viewTemplateId"),
    ("DBViewPlan", "ELEM_FAMILY_AND_TYPE_PARAM", "m_dbViewTypeId"),
    ("DBViewPlan", "ELEM_FAMILY_PARAM", "m_dbViewTypeId"),
    ("DBViewPlan", "ELEM_TYPE_PARAM", "m_dbViewTypeId"),
    ("DBViewPlan", "SYMBOL_ID_PARAM", "m_dbViewTypeId"),
    (
        "DBViewPlan",
        "VIEW_ANALYSIS_DISPLAY_STYLE",
        "m_analysisDisplayStyleId",
    ),
    ("DBViewPlan", "VIEW_DESCRIPTION", "m_viewDescription"),
    (
        "DBViewPlan",
        "VIEW_FIXED_SKETCH_PLANE",
        "m_fixedSketchPlaneId",
    ),
    (
        "DBViewPlan",
        "VIEW_MODEL_DISPLAY_MODE",
        "m_modelDisplayMode",
    ),
    ("DBViewPlan", "VIEW_NAME", "m_viewName"),
    ("DBViewPlan", "VIEW_PARTS_VISIBILITY", "m_partsVisibility"),
    (
        "DBViewPlan",
        "VIEW_SLANTED_COLUMN_SYMBOL_OFFSET",
        "m_columnSymbolElev",
    ),
    ("DBViewPlan", "VIEW_TEMPLATE", "m_viewTemplateId"),
    (
        "DBViewPlan",
        "VIEW_TEMPLATE_FOR_SCHEDULE",
        "m_viewTemplateId",
    ),
    ("DBViewPlan", "VIEW_UNDERLAY_BOTTOM_ID", "m_idUnderlayLevel"),
    (
        "DBViewPlan",
        "VIEW_UNDERLAY_ORIENTATION",
        "m_underlayOrientation",
    ),
    ("DBViewPlan", "VIEW_UNDERLAY_TOP_ID", "m_idUnderlayTopLevel"),
    ("DBViewProject", "VIEW_DESCRIPTION", "m_viewDescription"),
    (
        "DBViewProject",
        "VIEW_FIXED_SKETCH_PLANE",
        "m_fixedSketchPlaneId",
    ),
    ("DBViewProject", "VIEW_NAME", "m_viewName"),
    (
        "DBViewProject",
        "VIEW_TEMPLATE_FOR_SCHEDULE",
        "m_viewTemplateId",
    ),
    (
        "DBViewSection",
        "VIEW_ANALYSIS_DISPLAY_STYLE",
        "m_analysisDisplayStyleId",
    ),
    ("DBViewSection", "VIEW_DESCRIPTION", "m_viewDescription"),
    (
        "DBViewSection",
        "VIEW_FIXED_SKETCH_PLANE",
        "m_fixedSketchPlaneId",
    ),
    (
        "DBViewSection",
        "VIEW_MODEL_DISPLAY_MODE",
        "m_modelDisplayMode",
    ),
    ("DBViewSection", "VIEW_NAME", "m_viewName"),
    (
        "DBViewSection",
        "VIEW_PARTS_VISIBILITY",
        "m_partsVisibility",
    ),
    ("DBViewSection", "VIEW_TEMPLATE", "m_viewTemplateId"),
    (
        "DBViewSection",
        "VIEW_TEMPLATE_FOR_SCHEDULE",
        "m_viewTemplateId",
    ),
    (
        "DBViewType",
        "ASSIGN_TEMPLATE_ON_VIEW_CREATION",
        "m_defaultTemplateIdAssignOnViewCreation",
    ),
    ("DBViewType", "CALLOUT_TAG", "m_calloutAttrId"),
    ("DBViewType", "DEFAULT_VIEW_TEMPLATE", "m_defaultTemplateId"),
    ("DBViewType", "ELEVATN_TAG", "m_elevatnAttrId"),
    ("DBViewType", "PLAN_VIEW_VIEW_DIR", "m_planViewDirection"),
    ("DBViewType", "POCHE_MAT_ID", "m_pocheMatId"),
    ("DBViewType", "SECTION_TAG", "m_sectionAttrId"),
    ("DimensionStyle", "ALTERNATE_UNITS", "m_alternateUnits"),
    (
        "DimensionStyle",
        "ALTERNATE_UNITS_PREFIX",
        "m_alternateUnitsPrefix",
    ),
    (
        "DimensionStyle",
        "ALTERNATE_UNITS_SUFFIX",
        "m_alternateUnitsSuffix",
    ),
    ("DimensionStyle", "ARC_CENTER_MARK", "m_arcCenterMark"),
    (
        "DimensionStyle",
        "DIAMETER_SYMBOL_LOCATION",
        "m_radiusDiameterSymbolLocation",
    ),
    (
        "DimensionStyle",
        "DIAMETER_SYMBOL_TEXT",
        "m_radiusDiameterPrefixText",
    ),
    (
        "DimensionStyle",
        "DIM_LEADER_ARROWHEAD",
        "m_arrowHeadStyleId",
    ),
    (
        "DimensionStyle",
        "DIM_LEADER_DISPLAY_CONDITION",
        "m_leaderDisplayCondition",
    ),
    (
        "DimensionStyle",
        "DIM_LEADER_SHOULDER_LENGTH",
        "m_leaderShoulderLength",
    ),
    ("DimensionStyle", "DIM_LEADER_TYPE", "m_leaderType"),
    (
        "DimensionStyle",
        "DIM_STYLE_CENTERLINE_TICK_MARK",
        "m_centerlineTickMarkStyleId",
    ),
    (
        "DimensionStyle",
        "DIM_STYLE_DIM_LINE_SNAP_DIST",
        "m_dimLineSnapDist",
    ),
    (
        "DimensionStyle",
        "DIM_STYLE_INTERIOR_TICK_MARK",
        "m_interiorTickMarkStyleId",
    ),
    (
        "DimensionStyle",
        "DIM_STYLE_LEADER_TICK_MARK",
        "m_leaderTickMarkStyleId",
    ),
    (
        "DimensionStyle",
        "DIM_STYLE_SHOW_OPENING_HT",
        "m_showOpeningHt",
    ),
    (
        "DimensionStyle",
        "DIM_STYLE_SUPPRESS_SPACES",
        "m_suppressSpaces",
    ),
    (
        "DimensionStyle",
        "DIM_TEXT_LOCATION_FOR_LEADER",
        "m_leaderTextLocation",
    ),
    (
        "DimensionStyle",
        "DIM_WITNS_LINE_CNTRL",
        "m_dimWitnCntrlExtBelow",
    ),
    (
        "DimensionStyle",
        "EQUALITY_TEXT_FOR_ANGULAR_DIM",
        "m_equalityText",
    ),
    (
        "DimensionStyle",
        "EQUALITY_TEXT_FOR_CONTINUOUS_LINEAR_DIM",
        "m_equalityText",
    ),
    (
        "DimensionStyle",
        "EQUALITY_WITNESS_DISPLAY",
        "m_equalityWitnessDisplay",
    ),
    (
        "DimensionStyle",
        "INTERIOR_TICK_DISPLAY",
        "m_interiorTickMarkDisplay",
    ),
    ("DimensionStyle", "LINEAR_DIM_TYPE", "m_linearDimType"),
    (
        "DimensionStyle",
        "RADIUS_SYMBOL_LOCATION",
        "m_radiusDiameterSymbolLocation",
    ),
    (
        "DimensionStyle",
        "RADIUS_SYMBOL_TEXT",
        "m_radiusDiameterPrefixText",
    ),
    (
        "DimensionStyle",
        "WITNS_LINE_TICK_MARK",
        "m_witnessLineTickMarkStyleId",
    ),
    ("DivisionRule", "DIVISION_PATTERN", "m_patternId"),
    (
        "EnergyDataSettings",
        "ENERGY_ANALYSIS_CURRENT_VIEW_ONLY",
        "m_useCurrentViewOnly",
    ),
    (
        "EnergyDataSettings",
        "ENERGY_ANALYSIS_HORIZONTAL_VOID_THRESHOLD",
        "m_horizontalVoidThreshold",
    ),
    (
        "EnergyDataSettings",
        "ENERGY_ANALYSIS_VERTICAL_VOID_THRESHOLD",
        "m_verticalVoidThreshold",
    ),
    (
        "EnergyDataSettings",
        "RBS_ENERGY_ANALYSIS_BUILDING_ENVELOPE_ANALYTICAL_SPACE_IDENTIFICATION_RESOLUTION",
        "m_spaceResolution",
    ),
    (
        "EnergyDataSettings",
        "RBS_ENERGY_ANALYSIS_BUILDING_ENVELOPE_ANALYTICAL_SURFACE_IDENTIFICATION_RESOLUTION",
        "m_surfaceResolution",
    ),
    (
        "EnergyDataSettings",
        "RBS_ENERGY_ANALYSIS_GROUND_PLANE_PARAM",
        "m_groundPlane",
    ),
    (
        "EnergyDataSettings",
        "RBS_ENERGY_ANALYSIS_PROJECT_PHASE_PARAM",
        "m_projectPhase",
    ),
    (
        "Family",
        "FAMILY_ALLOW_CUT_WITH_VOIDS",
        "m_bAllowCutWithUnattachedVoids",
    ),
    ("Family", "FAMILY_ALWAYS_VERTICAL", "m_isVertical"),
    ("Family", "FAMILY_CAN_HOST_REBAR", "m_canHostRebar"),
    ("Family", "FAMILY_CONTENT_PART_TYPE", "m_partType"),
    (
        "Family",
        "FAMILY_ELECTRICAL_MAINTAIN_ANNOTATION_ORIENTATION",
        "m_bElecKeepAnnotationOrientation",
    ),
    (
        "Family",
        "FAMILY_ENABLE_CUTTING_IN_VIEWS",
        "m_enableCuttingInViews",
    ),
    (
        "Family",
        "FAMILY_IS_ELEVATION_MARK_BODY",
        "m_bElevationMarkBody",
    ),
    ("Family", "FAMILY_KEEP_TEXT_READABLE", "m_keepTextReadable"),
    ("Family", "FAMILY_RENDERING_TYPE", "m_eRenderModelType"),
    (
        "Family",
        "FAMILY_ROTATE_TEXT_WITH_COMPONENT",
        "m_rotateTagTextWithComponent",
    ),
    (
        "Family",
        "FAMILY_ROTATE_WITH_COMPONENT",
        "m_rotateTagWithComponent",
    ),
    ("Family", "FAMILY_SHARED", "m_bShared"),
    (
        "Family",
        "FAMILY_STRUCT_MATERIAL_TYPE",
        "m_eStructMaterialType",
    ),
    ("Family", "FAMILY_SYMBOLIC_REP", "m_eSymbolicRepType"),
    ("Family", "FAMILY_USE_PRECUT_SHAPE", "m_bUsePreCutShape"),
    ("Family", "FAMILY_WORK_PLANE_BASED", "m_isWorkPlaneBased"),
    (
        "Family",
        "FAM_PROFILE_DEFINITION",
        "m_eProfileFamDefinition",
    ),
    ("Family", "FAM_PROFILE_USAGE", "m_eProfileFamUsage"),
    ("Family", "OMNICLASS_CODE", "m_omniClassCode"),
    (
        "Family",
        "ROOM_CALCULATION_POINT",
        "m_hasRoomCalculationPoint",
    ),
    (
        "Family",
        "STRUCTURAL_BEAM_CUTBACK_FOR_COLUMN",
        "m_eBeamCutbackForColumn",
    ),
    (
        "Family",
        "STRUCTURAL_FAMILY_CODE_NAME",
        "m_structuralCodeName",
    ),
    (
        "FilledRegionAttributes",
        "BACKGROUND_DRAFT_PATTERN_ID_PARAM",
        "m_backgroundPatternElemId",
    ),
    (
        "FilledRegionAttributes",
        "BACKGROUND_PATTERN_COLOR_PARAM",
        "m_backgroundPatternColor",
    ),
    (
        "FilledRegionAttributes",
        "FOREGROUND_ANY_PATTERN_ID_PARAM",
        "m_fillPatternElemId",
    ),
    (
        "FilledRegionAttributes",
        "FOREGROUND_DRAFT_PATTERN_ID_PARAM",
        "m_fillPatternElemId",
    ),
    (
        "FloorAttributes",
        "COARSE_SCALE_FILL_PATTERN_COLOR",
        "m_pCompoundStructure.m_coarseScaleFillColor",
    ),
    (
        "FloorAttributes",
        "COARSE_SCALE_FILL_PATTERN_ID_PARAM",
        "m_pCompoundStructure.m_coarseScaleFillPatternElemId",
    ),
    (
        "GeoLocation",
        "ELEM_FAMILY_AND_TYPE_PARAM",
        "m_pInstanceInfo.m_symbolId",
    ),
    (
        "GeoLocation",
        "ELEM_FAMILY_PARAM",
        "m_pInstanceInfo.m_symbolId",
    ),
    (
        "GeoLocation",
        "ELEM_TYPE_PARAM",
        "m_pInstanceInfo.m_symbolId",
    ),
    (
        "GeoLocation",
        "SYMBOL_ID_PARAM",
        "m_pInstanceInfo.m_symbolId",
    ),
    (
        "GridAttributes",
        "DATUM_BUBBLE_LOCATION_IN_ELEV",
        "m_bubbleLocInElev",
    ),
    ("GridAttributes", "GRID_BUBBLE_END_1", "m_bubbleAtEnd1"),
    ("GridAttributes", "GRID_BUBBLE_END_2", "m_bubbleAtEnd2"),
    (
        "GridAttributes",
        "GRID_CENTER_SEGMENT_STYLE",
        "m_centerSegmentStyle",
    ),
    (
        "GridAttributes",
        "GRID_END_SEGMENTS_LENGTH",
        "m_endSegmentsLength",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "BUILDING_CLOSING_TIME_PARAM",
        "m_strEquipmentEndTime",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "BUILDING_OPENING_TIME_PARAM",
        "m_strEquipmentStartTime",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "BUILDING_UNOCCUPIED_COOLING_SET_POINT_PARAM",
        "m_dCoolingSetPoint",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_AIR_CHANGES_PER_HOUR_PARAM",
        "m_dAirChangesPerHour",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_AREA_PER_PERSON_PARAM",
        "m_dAreaPerPerson",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_DESIGN_LIGHTING_LOAD_PER_AREA_PARAM",
        "m_LightingLoadDensity",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_DESIGN_POWER_LOAD_PER_AREA_PARAM",
        "m_PowerLoadDensity",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_OUTDOOR_AIRFLOW_STANDARD_PARAM",
        "m_eOutdoorAirFlowStandard",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_OUTDOOR_AIR_PER_AREA_PARAM",
        "m_dOutdoorAirPerArea",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_OUTDOOR_AIR_PER_PERSON_PARAM",
        "m_dOutdoorAirPerPerson",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_PEOPLE_LATENT_HEAT_GAIN_PER_PERSON_PARAM",
        "m_dLatentHeatGainPerPerson",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_PEOPLE_SENSIBLE_HEAT_GAIN_PER_PERSON_PARAM",
        "m_dSensibleHeatGainPerPerson",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "ROOM_PLENUM_LIGHTING_PARAM",
        "m_dPlenumLighting",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "SPACE_COOLING_SET_POINT",
        "m_coolingSetPoint",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "SPACE_DEHUMIDIFICATION_SET_POINT",
        "m_dehumidificationSetPoint",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "SPACE_HEATING_SET_POINT",
        "m_heatingSetPoint",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "SPACE_HUMIDIFICATION_SET_POINT",
        "m_humidificationSetPoint",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "SPACE_LIGHTING_SCHEDULE_PARAM",
        "m_idLightingSchedule",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "SPACE_OCCUPANCY_SCHEDULE_PARAM",
        "m_idOccupancySchedule",
    ),
    (
        "HVACLoadBuildingTypeElem",
        "SPACE_POWER_SCHEDULE_PARAM",
        "m_idPowerSchedule",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_AIR_CHANGES_PER_HOUR_PARAM",
        "m_dAirChangesPerHour",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_AREA_PER_PERSON_PARAM",
        "m_dAreaPerPerson",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_DESIGN_LIGHTING_LOAD_PER_AREA_PARAM",
        "m_LightingLoadDensity",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_DESIGN_POWER_LOAD_PER_AREA_PARAM",
        "m_PowerLoadDensity",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_OUTDOOR_AIRFLOW_STANDARD_PARAM",
        "m_eOutdoorAirFlowStandard",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_OUTDOOR_AIR_PER_AREA_PARAM",
        "m_dOutdoorAirPerArea",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_OUTDOOR_AIR_PER_PERSON_PARAM",
        "m_dOutdoorAirPerPerson",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_PEOPLE_LATENT_HEAT_GAIN_PER_PERSON_PARAM",
        "m_dLatentHeatGainPerPerson",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_PEOPLE_SENSIBLE_HEAT_GAIN_PER_PERSON_PARAM",
        "m_dSensibleHeatGainPerPerson",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "ROOM_PLENUM_LIGHTING_PARAM",
        "m_dPlenumLighting",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "SPACE_COOLING_SET_POINT",
        "m_coolingSetPoint",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "SPACE_DEHUMIDIFICATION_SET_POINT",
        "m_dehumidificationSetPoint",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "SPACE_HEATING_SET_POINT",
        "m_heatingSetPoint",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "SPACE_HUMIDIFICATION_SET_POINT",
        "m_humidificationSetPoint",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "SPACE_INFILTRATION_PARAM",
        "m_dInfiltration",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "SPACE_LIGHTING_SCHEDULE_PARAM",
        "m_idLightingSchedule",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "SPACE_OCCUPANCY_SCHEDULE_PARAM",
        "m_idOccupancySchedule",
    ),
    (
        "HVACLoadSpaceTypeElem",
        "SPACE_POWER_SCHEDULE_PARAM",
        "m_idPowerSchedule",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_BEGINNING_TERMINATION_ATTACHMENT_PARAM",
        "m_beginningAttachment",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_BEGINNING_TERMINATION_TYPE_PARAM",
        "m_beginningTermination",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_DEFAULT_JOIN_TYPE_PARAM",
        "m_defaultContinuousRailJoinType",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_END_EXTENSION_LENGTH_PARAM",
        "m_endExtensionLength",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_END_TERMINATION_ATTACHMENT_PARAM",
        "m_endAttachment",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_END_TERMINATION_TYPE_PARAM",
        "m_endTermination",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_EXTENSION_LENGTH_PARAM",
        "m_startExtensionLength",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_FILLET_RADIUS_PARAM",
        "m_filletRadius",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_PLUS_TREAD_DEPTH_PARAM",
        "m_plusOneTreadDepth",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_PROFILE_TYPE_PARAM",
        "m_profileId",
    ),
    (
        "HandRailType",
        "CONTINUOUSRAIL_TRANSITION_TYPE_PARAM",
        "m_transitionType",
    ),
    ("ImageSymbol", "RASTER_SYMBOL_HEIGHT", "m_heightInFeet"),
    (
        "ImageSymbol",
        "RASTER_SYMBOL_PIXELHEIGHT",
        "m_heightInPixels",
    ),
    ("ImageSymbol", "RASTER_SYMBOL_PIXELWIDTH", "m_widthInPixels"),
    ("ImageSymbol", "RASTER_SYMBOL_WIDTH", "m_widthInFeet"),
    ("InteriorElevArrow", "ELEV_ASSOC_DATUM", "m_assocDatumId"),
    (
        "InteriorElevArrow",
        "VIEWER_ANNOTATION_CROP_ACTIVE",
        "m_bAnnotationCropActive",
    ),
    (
        "InteriorElevAttributes",
        "ELEV_SYMBOL_ID",
        "m_elevationSymbolId",
    ),
    ("LeaderStyle", "ARROW_CENTERED", "m_arrowCentered"),
    ("LeaderStyle", "ARROW_CLOSED", "m_arrowClosed"),
    ("LeaderStyle", "ARROW_FILLED", "m_arrowFilled"),
    ("LeaderStyle", "ARROW_TYPE", "m_tickType"),
    ("Level", "DATUM_TEXT", "m_text"),
    ("Level", "LEVEL_IS_BUILDING_STORY", "m_isBuildingStory"),
    (
        "Level",
        "LEVEL_ROOM_COMPUTATION_HEIGHT",
        "m_roomComputationElevationOffset",
    ),
    ("MaterialElem", "MATERIAL_NAME", "m_pMaterial.m_name"),
    (
        "MaterialElem",
        "MATERIAL_PARAM_COLOR",
        "m_pMaterial.m_color",
    ),
    (
        "MaterialElem",
        "MATERIAL_PARAM_SHININESS",
        "m_pMaterial.m_shininess",
    ),
    ("NewCurtainWallType", "ALLOW_AUTO_EMBED", "m_allowAutoEmbed"),
    (
        "NewCurtainWallType",
        "AUTO_JOIN_CONDITION_WALL",
        "m_autoJoinCond",
    ),
    ("NewCurtainWallType", "AUTO_PANEL_WALL", "m_panel"),
    (
        "NewCurtainWallType",
        "SPACING_LENGTH_HORIZ",
        "m_oULineSpec.m_spacing",
    ),
    (
        "NewCurtainWallType",
        "SPACING_LENGTH_VERT",
        "m_oVLineSpec.m_spacing",
    ),
    ("ProfileRoof", "PHASE_CREATED", "m_createdPhaseId"),
    ("ProfileRoof", "PHASE_DEMOLISHED", "m_demolishedPhaseId"),
    ("ProfileRoof", "WALL_ATTR_ROOM_BOUNDING", "m_roomBounding"),
    ("ProjectPhase", "PHASE_NAME", "m_name"),
    (
        "RbsCableTrayType",
        "RBS_CURVETYPE_DEFAULT_CROSS_PARAM",
        "m_idDefaultCross",
    ),
    (
        "RbsCableTrayType",
        "RBS_CURVETYPE_DEFAULT_TEE_PARAM",
        "m_idDefaultTee",
    ),
    (
        "RbsCableTrayType",
        "RBS_CURVETYPE_DEFAULT_TRANSITION_PARAM",
        "m_idDefaultTransition",
    ),
    (
        "RbsCableTrayType",
        "RBS_CURVETYPE_DEFAULT_UNION_PARAM",
        "m_idDefaultUnion",
    ),
    (
        "RbsConduitType",
        "CONDUIT_STANDARD_TYPE_PARAM",
        "m_idStandardType",
    ),
    (
        "RbsConduitType",
        "RBS_CURVETYPE_DEFAULT_CROSS_PARAM",
        "m_idDefaultCross",
    ),
    (
        "RbsConduitType",
        "RBS_CURVETYPE_DEFAULT_TEE_PARAM",
        "m_idDefaultTee",
    ),
    (
        "RbsConduitType",
        "RBS_CURVETYPE_DEFAULT_TRANSITION_PARAM",
        "m_idDefaultTransition",
    ),
    (
        "RbsConduitType",
        "RBS_CURVETYPE_DEFAULT_UNION_PARAM",
        "m_idDefaultUnion",
    ),
    (
        "RbsDbViewSystemNavigator",
        "VIEW_DESCRIPTION",
        "m_viewDescription",
    ),
    (
        "RbsDbViewSystemNavigator",
        "VIEW_FIXED_SKETCH_PLANE",
        "m_fixedSketchPlaneId",
    ),
    ("RbsDbViewSystemNavigator", "VIEW_NAME", "m_viewName"),
    (
        "RbsDbViewSystemNavigator",
        "VIEW_TEMPLATE_FOR_SCHEDULE",
        "m_viewTemplateId",
    ),
    (
        "RbsFlexPipeType",
        "RBS_CURVETYPE_DEFAULT_TAKEOFF_PARAM",
        "m_idDefaultTakeoff",
    ),
    (
        "RbsFlexPipeType",
        "RBS_CURVETYPE_DEFAULT_TEE_PARAM",
        "m_idDefaultTee",
    ),
    (
        "RbsFlexPipeType",
        "RBS_CURVETYPE_DEFAULT_TRANSITION_PARAM",
        "m_idDefaultTransition",
    ),
    (
        "RbsFlexPipeType",
        "RBS_CURVETYPE_DEFAULT_UNION_PARAM",
        "m_idDefaultUnion",
    ),
    (
        "RbsHvacSystemType",
        "RBS_SYSTEM_RISEDROP_PARAM",
        "m_RiseDropSettings",
    ),
    (
        "RbsPipingSystemType",
        "RBS_PIPE_FLUID_TEMPERATURE_PARAM",
        "m_fluidTemperature",
    ),
    (
        "RbsPipingSystemType",
        "RBS_PIPE_FLUID_TYPE_PARAM",
        "m_fluidType",
    ),
    (
        "RbsPipingSystemType",
        "RBS_PIPE_SYSTEM_CALCULATION_PARAM",
        "m_calculationLevel",
    ),
    (
        "RbsPipingSystemType",
        "RBS_SYSTEM_FLOW_CONVERSION_METHOD_PARAM",
        "m_flowConversionMethod",
    ),
    (
        "RbsWireType",
        "RBS_ELEC_WIRE_SHARE_GROUND",
        "m_bShareGround",
    ),
    (
        "RbsWireType",
        "RBS_ELEC_WIRE_SHARE_NEUTRAL",
        "m_bShareNeutral",
    ),
    (
        "RbsWireType",
        "RBS_WIRE_CONDUIT_TYPE_PARAM",
        "m_strConduitType",
    ),
    (
        "RbsWireType",
        "RBS_WIRE_MAX_CONDUCTOR_SIZE_PARAM",
        "m_strMaxConductorSize",
    ),
    (
        "RbsWireType",
        "RBS_WIRE_NEUTRAL_INCLUDED_IN_BALANCED_LOAD_PARAM",
        "m_bNeutralIncludedInBalancedLoad",
    ),
    (
        "RbsWireType",
        "RBS_WIRE_NEUTRAL_MODE_PARAM",
        "m_eNeutralMode",
    ),
    (
        "RbsWireType",
        "RBS_WIRE_NEUTRAL_MULTIPLIER_PARAM",
        "m_dNeutralMultiplier",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_ANGULAR_DIMENSION_TEXT_POSITION",
        "m_angularDimensionTextPosition",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_DETAIL_LEVEL",
        "m_detailLevel",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_SEGMENT_LENGTH_DIMENSION_TEXT_POSITION",
        "m_segmentLengthDimensionTextPosition",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_ANGULAR_DIMENSIONS_ENABLED",
        "m_angularDimensionsEnabled",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_ANGULAR_DIMENSIONS_FOR_HOOKS_ENABLED",
        "m_angularDimensionsForHooksEnabled",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_ANGULAR_DIMENSIONS_MEASUREMENT",
        "m_angularDimensionsMeasurementOptions",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_ANGULAR_DIMENSION_OFFSET",
        "m_angularDimensionsOffset",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_ANGULAR_DIMENSION_TYPE_ID",
        "m_angularDimensionTypeId",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_BEND_DIAMETER_DIMENSIONS_ENABLED",
        "m_bendDiameterDimensionsEnabled",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_BEND_DIAMETER_DIMENSIONS_FOR_HOOKS_ENABLED",
        "m_bendDiameterDimensionsForHooksEnabled",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_BEND_DIAMETER_DIMENSIONS_FOR_SEGMENTS_ENABLED",
        "m_bendDiameterDimensionsForSegmentsEnabled",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_DIAMETER_DIMENSION_TYPE_ID",
        "m_diameterDimensionTypeId",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_ORTHOGONAL_AND_OVERALL_DIMESIONS_ENABLED",
        "m_orthogonalAndOverallDimesionsEnabled",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_RADIAL_DIMENSION_TYPE_ID",
        "m_radialDimensionTypeId",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_SEGMENT_LENGTHS_DISPLAY_OPTION",
        "m_segmentLengthsDisplayOption",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_SEGMENT_LENGTHS_FOR_ARCS_DISPLAY_OPTION",
        "m_segmentLengthsForArcsDisplayOption",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_SEGMENT_LENGTH_DIMENSIONS_ENABLED",
        "m_segmentLengthDimensionsEnabled",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_SEGMENT_LENGTH_DIMENSIONS_FOR_HOOKS_ENABLED",
        "m_segmentLengthDimensionsForHooksEnabled",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_SEGMENT_LENGTH_DIMENSIONS_OFFSET",
        "m_segmentLengthDimensionsOffset",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_SEGMENT_LENGTH_DIMENSION_TYPE_ID",
        "m_segmentLengthDimensionTypeId",
    ),
    (
        "RebarBendingDetailType",
        "BENDING_DETAIL_TYPE_SHOW_ANGULAR_DIMENSIONS_FOR",
        "m_angularDimensionsDisplayOption",
    ),
    (
        "RevealAttr",
        "WALL_SWEEP_DEFAULT_SETBACK_PARAM",
        "m_defaultSetback",
    ),
    (
        "RoofAttributes",
        "COARSE_SCALE_FILL_PATTERN_COLOR",
        "m_pCompoundStructure.m_coarseScaleFillColor",
    ),
    (
        "RoofAttributes",
        "COARSE_SCALE_FILL_PATTERN_ID_PARAM",
        "m_pCompoundStructure.m_coarseScaleFillPatternElemId",
    ),
    ("SWall", "PHASE_CREATED", "m_createdPhaseId"),
    ("SWall", "PHASE_DEMOLISHED", "m_demolishedPhaseId"),
    ("SWall", "WALL_ATTR_ROOM_BOUNDING", "m_roomBounding"),
    ("SWall", "WALL_CROSS_SECTION", "m_wallCrossSection"),
    ("SWall", "WALL_HEIGHT_TYPE", "m_subWallGridIdInType"),
    ("SWall", "WALL_KEY_REF_PARAM", "m_wallKeyRef"),
    (
        "SWall",
        "WALL_STRUCTURAL_SIGNIFICANT",
        "m_isStructuralSignificant",
    ),
    (
        "SectionAttributes",
        "SECTION_ATTR_HEAD_TAG",
        "m_sectionHeadFamilyTagId",
    ),
    (
        "SectionAttributes",
        "SECTION_ATTR_TAIL_TAG",
        "m_sectionTailFamilyTagId",
    ),
    (
        "SectionAttributes",
        "SECTION_BROKEN_DISPLAY_STYLE",
        "m_styleForBrokenDisplay",
    ),
    (
        "SpotElevationStyle",
        "ALIGNMENT_STATION_LABEL_INCLUDE_STATION",
        "m_aslIncludeStation",
    ),
    (
        "SpotElevationStyle",
        "ALIGNMENT_STATION_LABEL_IND_STATION",
        "m_aslIndStation",
    ),
    ("SpotElevationStyle", "ALTERNATE_UNITS", "m_alternateUnits"),
    (
        "SpotElevationStyle",
        "ALTERNATE_UNITS_PREFIX",
        "m_alternateUnitsPrefix",
    ),
    (
        "SpotElevationStyle",
        "ALTERNATE_UNITS_SUFFIX",
        "m_alternateUnitsSuffix",
    ),
    (
        "SpotElevationStyle",
        "DIM_STYLE_SUPPRESS_SPACES",
        "m_suppressSpaces",
    ),
    (
        "SpotElevationStyle",
        "SPOT_COORDINATE_INCLUDE_ELEVATION",
        "m_seIncludeElevation",
    ),
    ("SpotElevationStyle", "SPOT_ELEV_BASE", "m_seBase"),
    ("SpotElevationStyle", "SPOT_ELEV_BOT_VALUE", "m_seBotValue"),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_IND_BOTTOM",
        "m_seIndBottom",
    ),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_IND_ELEVATION",
        "m_seIndElevation",
    ),
    ("SpotElevationStyle", "SPOT_ELEV_IND_TOP", "m_seIndTop"),
    ("SpotElevationStyle", "SPOT_ELEV_IND_TYPE", "m_seIndType"),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_IND_TYPE_BOTTOM",
        "m_seIndTypeBottom",
    ),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_IND_TYPE_ELEVATION",
        "m_seIndType",
    ),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_IND_TYPE_TOP",
        "m_seIndTypeTop",
    ),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_LEADER_ARROWHEAD",
        "m_arrowHeadStyleId",
    ),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_ROTATE_WITH_COMPONENT",
        "m_seRotateWithComponent",
    ),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_TEXT_HORIZ_OFFSET",
        "m_seHorizOffset",
    ),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_TEXT_LOCATION",
        "m_seTextLoc",
    ),
    (
        "SpotElevationStyle",
        "SPOT_ELEV_TEXT_ORIENTATION",
        "m_seTextOrientation",
    ),
    ("SpotElevationStyle", "SPOT_ELEV_TOP_VALUE", "m_seTopValue"),
    (
        "SpotElevationStyle",
        "SPOT_SLOPE_LEADER_LENGTH",
        "m_seLeaderLength",
    ),
    (
        "StairsLandingType",
        "STAIRS_LANDINGTYPE_LANDING_MATERIAL",
        "m_landingMaterialId",
    ),
    (
        "StairsLandingType",
        "STAIRS_LANDINGTYPE_THICKNESS",
        "m_thickness",
    ),
    (
        "StairsLandingType",
        "STAIRS_LANDINGTYPE_USE_SAME_TRISER_AS_RUN",
        "m_useSameTriserAsRun",
    ),
    ("StairsPathType", "ARROWHEAD_END_AT_RISER", "m_endAtRiser"),
    ("StairsPathType", "ARROWHEAD_TYPE", "m_arrowheadTypeId"),
    (
        "StairsPathType",
        "DISTANCE_TO_CUT_MARK",
        "m_distanceToCutMark",
    ),
    ("StairsPathType", "DRAW_FOR_EACH_RUN", "m_drawForEachRun"),
    (
        "StairsPathType",
        "LINE_SHAPE_AT_CORNER",
        "m_lineShapeAtCorner",
    ),
    (
        "StairsPathType",
        "SHOW_ARROWHEAD_TO_CUT_MARK",
        "m_showArrowheadToCutMark",
    ),
    (
        "StairsPathType",
        "STAIRS_PATH_FULL_STEP_ARROW",
        "m_fullStepArrow",
    ),
    (
        "StairsPathType",
        "STAIRS_PATH_START_EXTENSION",
        "m_startExtension",
    ),
    (
        "StairsPathType",
        "STAIRS_PATH_START_FROM_RISER",
        "m_startFromRiser",
    ),
    ("StairsPathType", "STAIRS_TEXT_TYPE", "m_textTypeId"),
    ("StairsPathType", "START_SYMBOL_TYPE", "m_startSymbolTypeId"),
    (
        "StairsRailingAttr",
        "RAILING_SYSTEM_TOP_RAIL_HEIGHT_PARAM",
        "m_topRailHeight",
    ),
    (
        "StairsRailingAttr",
        "RAILING_SYSTEM_TOP_RAIL_TYPES_PARAM",
        "m_topRailTypeId",
    ),
    (
        "StairsRunType",
        "STAIRS_RUNTYPE_RUN_MATERIAL",
        "m_runMaterialId",
    ),
    (
        "StairsRunType",
        "STAIRS_RUNTYPE_STRUCTURAL_DEPTH",
        "m_structuralDepth",
    ),
    (
        "StairsRunType",
        "STAIRS_RUNTYPE_UNDERSIDE_SURFACE_TYPE",
        "m_undersideSurfaceType",
    ),
    (
        "StairsRunType",
        "STAIRS_TRISERTYPE_NOSING_LENGTH",
        "m_nosingLength",
    ),
    (
        "StairsRunType",
        "STAIRS_TRISERTYPE_NOSING_PLACEMENT",
        "m_nosingPlacement",
    ),
    (
        "StairsRunType",
        "STAIRS_TRISERTYPE_NOSING_PROFILE",
        "m_nosingProfileTypeId",
    ),
    ("StairsRunType", "STAIRS_TRISERTYPE_RISER", "m_hasRiser"),
    (
        "StairsRunType",
        "STAIRS_TRISERTYPE_RISER_IS_SLANTED",
        "m_riserIsSlanted",
    ),
    (
        "StairsRunType",
        "STAIRS_TRISERTYPE_RISER_MATERIAL",
        "m_riserMaterialId",
    ),
    (
        "StairsRunType",
        "STAIRS_TRISERTYPE_RISER_PROFILE",
        "m_riserProfileTypeId",
    ),
    (
        "StairsRunType",
        "STAIRS_TRISERTYPE_RISER_TREAD_CONNECTION",
        "m_riserTreadConnection",
    ),
    ("StairsRunType", "STAIRS_TRISERTYPE_TREAD", "m_hasTread"),
    (
        "StairsRunType",
        "STAIRS_TRISERTYPE_TREAD_MATERIAL",
        "m_treadMaterialId",
    ),
    (
        "StairsRunType",
        "STAIRS_TRISERTYPE_TREAD_PROFILE",
        "m_treadProfileTypeId",
    ),
    (
        "StairsSupportType",
        "STAIRS_SUPPORTTYPE_FLIP_SECTION_PROFILE",
        "m_flipSectionProfile",
    ),
    (
        "StairsSupportType",
        "STAIRS_SUPPORTTYPE_SECTION_PROFILE",
        "m_sectionProfileId",
    ),
    (
        "StairsSupportType",
        "STAIRS_SUPPORTTYPE_STRUCTURAL_DEPTH_ON_LANDING",
        "m_structuralDepthOnLanding",
    ),
    (
        "StairsSupportType",
        "STAIRS_SUPPORTTYPE_STRUCTURAL_DEPTH_ON_RUN",
        "m_structuralDepthOnRun",
    ),
    (
        "StairsSupportType",
        "STAIRS_SUPPORTTYPE_TOTAL_DEPTH",
        "m_totalDepth",
    ),
    (
        "StairsType",
        "STAIRSTYPE_GEOMUNJOINED_END_CUT_STYLE",
        "m_geomUnjoinedEndCutStyle",
    ),
    (
        "StairsType",
        "STAIRSTYPE_HAS_INTERMEDIATE_SUPPORT",
        "m_hasIntermediateSupport",
    ),
    (
        "StairsType",
        "STAIRSTYPE_HAS_LEFT_SUPPORT",
        "m_leftSupportStyle",
    ),
    (
        "StairsType",
        "STAIRSTYPE_HAS_RIGHT_SUPPORT",
        "m_rightSupportStyle",
    ),
    (
        "StairsType",
        "STAIRSTYPE_INTERMEDIATE_SUPPORT_TYPE",
        "m_intermediateSupportTypeId",
    ),
    ("StairsType", "STAIRSTYPE_LANDING_TYPE", "m_landingTypeId"),
    (
        "StairsType",
        "STAIRSTYPE_LEFT_SIDE_SUPPORT_TYPE",
        "m_leftSideSupportTypeId",
    ),
    (
        "StairsType",
        "STAIRSTYPE_LEFT_SUPPORT_LATERAL_OFFSET",
        "m_leftSupportLateralOffset",
    ),
    (
        "StairsType",
        "STAIRSTYPE_MINIMUM_RUN_WIDTH",
        "m_minRunWidth",
    ),
    (
        "StairsType",
        "STAIRSTYPE_NOTCH_EXTENSION",
        "m_notchExtension",
    ),
    (
        "StairsType",
        "STAIRSTYPE_NOTCH_HORIZONTAL_GAP",
        "m_notchHorizontalGap",
    ),
    (
        "StairsType",
        "STAIRSTYPE_NOTCH_THICKNESS",
        "m_notchThickness",
    ),
    (
        "StairsType",
        "STAIRSTYPE_NOTCH_VERTICAL_GAP",
        "m_notchVerticalGap",
    ),
    (
        "StairsType",
        "STAIRSTYPE_RIGHT_SIDE_SUPPORT_TYPE",
        "m_rightSideSupportTypeId",
    ),
    (
        "StairsType",
        "STAIRSTYPE_RIGHT_SUPPORT_LATERAL_OFFSET",
        "m_rightSupportLateralOffset",
    ),
    ("StairsType", "STAIRSTYPE_RUN_TYPE", "m_runTypeId"),
    (
        "StairsType",
        "STAIRS_ATTR_MAX_RISER_HEIGHT",
        "m_maxRiserHeight",
    ),
    (
        "StairsType",
        "STAIRS_ATTR_MINIMUM_TREAD_DEPTH",
        "m_minTreadDepth",
    ),
    (
        "StructConnectionType",
        "STRUCT_CONNECTION_TYPE_NAME",
        "m_symbolInfo.m_name",
    ),
    (
        "SunAndShadowSettings",
        "VIEW_GRAPH_SUN_PATH_SIZE",
        "m_sunPathSize",
    ),
    ("SysPanelFamSym", "CURTAIN_WALL_SYSPANEL_OFFSET", "m_offset"),
    (
        "SysPanelFamSym",
        "CURTAIN_WALL_SYSPANEL_THICKNESS",
        "m_thickness",
    ),
    ("SysPanelFamSym", "MATERIAL_ID_PARAM", "m_renderStyleId"),
    (
        "TagNoteAttributes",
        "LEADER_OFFSET_SHEET",
        "m_leaderOffsetSheet",
    ),
    (
        "TagNoteAttributes",
        "TEXT_BACKGROUND",
        "m_lineAndTextAttr.m_background",
    ),
    (
        "TagNoteAttributes",
        "TEXT_BOX_VISIBILITY",
        "m_textBoxVisibility",
    ),
    (
        "TagNoteAttributes",
        "TEXT_STYLE_BOLD",
        "m_lineAndTextAttr.m_bBold",
    ),
    (
        "TagNoteAttributes",
        "TEXT_STYLE_UNDERLINE",
        "m_lineAndTextAttr.m_bUnderline",
    ),
    (
        "TextNoteAttributes",
        "LEADER_OFFSET_SHEET",
        "m_leaderOffsetSheet",
    ),
    (
        "TextNoteAttributes",
        "TEXT_BOX_VISIBILITY",
        "m_textBoxVisibility",
    ),
    (
        "TextNoteAttributes",
        "TEXT_STYLE_ITALIC",
        "m_lineAndTextAttr.m_bItalic",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_BEGINNING_TERMINATION_ATTACHMENT_PARAM",
        "m_beginningAttachment",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_BEGINNING_TERMINATION_TYPE_PARAM",
        "m_beginningTermination",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_DEFAULT_JOIN_TYPE_PARAM",
        "m_defaultContinuousRailJoinType",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_END_EXTENSION_LENGTH_PARAM",
        "m_endExtensionLength",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_END_TERMINATION_ATTACHMENT_PARAM",
        "m_endAttachment",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_END_TERMINATION_TYPE_PARAM",
        "m_endTermination",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_EXTENSION_LENGTH_PARAM",
        "m_startExtensionLength",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_FILLET_RADIUS_PARAM",
        "m_filletRadius",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_PLUS_TREAD_DEPTH_PARAM",
        "m_plusOneTreadDepth",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_PROFILE_TYPE_PARAM",
        "m_profileId",
    ),
    (
        "TopRailType",
        "CONTINUOUSRAIL_TRANSITION_TYPE_PARAM",
        "m_transitionType",
    ),
    (
        "Viewer",
        "VIEWER_ANNOTATION_CROP_ACTIVE",
        "m_bAnnotationCropActive",
    ),
    ("Viewer", "VIEW_UNDERLAY_ORIENTATION", "m_bReflected"),
    (
        "ViewportAttributes",
        "VIEWPORT_ATTR_PRESERVE_TITLE_POSITION",
        "m_preserveTitlePosition",
    ),
    (
        "ViewportAttributes",
        "VIEWPORT_ATTR_SHOW_EXTENSION_LINE",
        "m_showExtensionLine",
    ),
    (
        "ViewportAttributes",
        "VIEWPORT_ATTR_SHOW_LABEL",
        "m_showLabel",
    ),
    (
        "ZoneElement",
        "ZONE_COIL_BYPASS_PERCENTAGE_PARAM",
        "m_dCoilBypassPercentage",
    ),
    (
        "ZoneElement",
        "ZONE_COOLING_AIR_TEMPERATURE_PARAM",
        "m_dCoolingAirTemperature",
    ),
    (
        "ZoneElement",
        "ZONE_COOLING_SET_POINT_PARAM",
        "m_dCoolingSetPoint",
    ),
    (
        "ZoneElement",
        "ZONE_DEHUMIDIFICATION_SET_POINT_PARAM",
        "m_dDehumidificationSetPoint",
    ),
    (
        "ZoneElement",
        "ZONE_HEATING_AIR_TEMPERATURE_PARAM",
        "m_dHeatingAirTemperature",
    ),
    (
        "ZoneElement",
        "ZONE_HEATING_SET_POINT_PARAM",
        "m_dHeatingSetPoint",
    ),
    (
        "ZoneElement",
        "ZONE_HUMIDIFICATION_SET_POINT_PARAM",
        "m_dHumidificationSetPoint",
    ),
    ("ZoneElement", "ZONE_SERVICE_TYPE_PARAM", "m_eServiceType"),
    (
        "ZoneElement",
        "ZONE_USE_AIR_CHANGES_PER_HOUR_PARAM",
        "m_bUseAirChangesPerHour",
    ),
    (
        "ZoneElement",
        "ZONE_USE_DEHUMIDIFICATION_SETPOINT_PARAM",
        "m_bUseDehumidificationSetPoint",
    ),
    (
        "ZoneElement",
        "ZONE_USE_HUMIDIFICATION_SETPOINT_PARAM",
        "m_bUseHumidificationSetPoint",
    ),
    (
        "ZoneElement",
        "ZONE_USE_OUTSIDE_AIR_PER_AREA_PARAM",
        "m_bUseAirChangesPerHour",
    ),
    (
        "ZoneElement",
        "ZONE_USE_OUTSIDE_AIR_PER_PERSON_PARAM",
        "m_bUseAirChangesPerHour",
    ),
];

/// `ELEM_CATEGORY_PARAM`.
pub const ELEM_CATEGORY_PARAM: i64 = -1_140_362;
/// `ELEM_CATEGORY_PARAM_MT`.
pub const ELEM_CATEGORY_PARAM_MT: i64 = -1_140_363;
/// `DESIGN_OPTION_ID`.
pub const DESIGN_OPTION_ID: i64 = -1_013_201;

fn id(fields: &Fields, name: &str) -> Option<i64> {
    match fields.get(name)? {
        Value::Id(id) => Some(*id),
        _ => None,
    }
}

/// The parameters an element header gives: the category twice and the
/// design option. Empty when the header holds neither field.
pub fn header_parameters(header: &Fields) -> Vec<(i64, ParamValue)> {
    let mut out = Vec::new();
    if let Some(category) = id(header, "m_categroryId") {
        out.push((ELEM_CATEGORY_PARAM, ParamValue::ElementId(category)));
        out.push((ELEM_CATEGORY_PARAM_MT, ParamValue::ElementId(category)));
    }
    if let Some(option) = id(header, "m_designOptionId") {
        let option = if option == -4 { -1 } else { option };
        out.push((DESIGN_OPTION_ID, ParamValue::ElementId(option)));
    }
    out
}

/// The value at `path` (field names joined by `.`) of the element object
/// `objects[0]`, following objects held by value and pointers into
/// `objects`.
fn at<'a>(objects: &'a [Fields], path: &str) -> Option<&'a Value> {
    let mut cur = objects.first()?;
    let mut parts = path.split('.').peekable();
    while let Some(name) = parts.next() {
        let v = cur.get(name)?;
        if parts.peek().is_none() {
            return Some(v);
        }
        cur = match v {
            Value::Object(inner) => inner,
            Value::New { index, .. } => objects.get(*index)?,
            _ => return None,
        };
    }
    None
}

/// A field value as a parameter of storage type `storage` (`Integer`,
/// `Double`, `String`, `ElementId`); `None` when the kinds do not fit.
fn as_parameter(v: &Value, storage: &str) -> Option<ParamValue> {
    Some(match (storage, v) {
        ("Integer", Value::Int(i)) => ParamValue::Integer(*i),
        ("Integer", Value::Bool(b)) => ParamValue::Integer(i64::from(*b)),
        ("Double", Value::Float(f)) => ParamValue::Double(*f),
        ("String", Value::Text(s)) => ParamValue::String(s.clone()),
        ("ElementId", Value::Id(i)) => ParamValue::ElementId(*i),
        _ => return None,
    })
}

/// The parameters an element of schema class `class` gives through
/// [`FIELD_RULES`]; `objects` are its data object's objects, the element
/// object first ([`crate::schema_objects::SchemaObject::objects`]).
pub fn field_parameters(class: &str, objects: &[Fields]) -> Vec<(i64, ParamValue)> {
    let at_class = FIELD_RULES.partition_point(|r| r.0 < class);
    FIELD_RULES[at_class..]
        .iter()
        .take_while(|r| r.0 == class)
        .filter_map(|&(_, name, path)| {
            let p = builtin_parameters::parameter_by_enum(name)?;
            Some((p.id, as_parameter(at(objects, path)?, p.storage_type)?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(values: Vec<(&str, Value)>) -> Fields {
        Fields {
            tag: 1,
            span: 0..0,
            spans: values.iter().map(|_| 0..0).collect(),
            values: values
                .into_iter()
                .map(|(n, v)| (n.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn rules_are_sorted_and_name_known_parameters() {
        assert!(
            FIELD_RULES
                .windows(2)
                .all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1))
        );
        for (_, name, _) in FIELD_RULES {
            let p = builtin_parameters::parameter_by_enum(name).expect(name);
            assert!(!p.storage_type.is_empty(), "{name} has no storage type");
        }
    }

    #[test]
    fn header_ids_are_the_catalog_s() {
        for (id, name) in [
            (ELEM_CATEGORY_PARAM, "ELEM_CATEGORY_PARAM"),
            (ELEM_CATEGORY_PARAM_MT, "ELEM_CATEGORY_PARAM_MT"),
            (DESIGN_OPTION_ID, "DESIGN_OPTION_ID"),
        ] {
            assert_eq!(
                builtin_parameters::parameter_by_enum(name).map(|p| p.id),
                Some(id)
            );
        }
    }

    #[test]
    fn header_gives_category_and_design_option() {
        let h = fields(vec![
            ("m_categroryId", Value::Id(-2_000_035)),
            ("m_designOptionId", Value::Id(-4)),
        ]);
        assert_eq!(
            header_parameters(&h),
            vec![
                (ELEM_CATEGORY_PARAM, ParamValue::ElementId(-2_000_035)),
                (ELEM_CATEGORY_PARAM_MT, ParamValue::ElementId(-2_000_035)),
                (DESIGN_OPTION_ID, ParamValue::ElementId(-1)),
            ]
        );
    }

    #[test]
    fn a_field_through_an_object_held_by_value() {
        let attr = fields(vec![("m_bItalic", Value::Bool(true))]);
        let root = fields(vec![("m_lineAndTextAttr", Value::Object(attr))]);
        let got = field_parameters("TextNoteAttributes", &[root]);
        let italic = builtin_parameters::parameter_by_enum("TEXT_STYLE_ITALIC")
            .unwrap()
            .id;
        assert!(got.contains(&(italic, ParamValue::Integer(1))), "{got:?}");
    }

    #[test]
    fn a_field_through_a_pointer() {
        let material = fields(vec![("m_shininess", Value::Int(64))]);
        let root = fields(vec![(
            "m_pMaterial",
            Value::New {
                tag: 2,
                index: 1,
                marker: 0,
            },
        )]);
        let got = field_parameters("MaterialElem", &[root, material]);
        let shininess = builtin_parameters::parameter_by_enum("MATERIAL_PARAM_SHININESS")
            .unwrap()
            .id;
        assert!(
            got.contains(&(shininess, ParamValue::Integer(64))),
            "{got:?}"
        );
    }
}
