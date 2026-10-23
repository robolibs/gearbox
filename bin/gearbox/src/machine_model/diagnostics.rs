//! Structured diagnostics of the machine compiler: a stable code, a severity
//! and the composed prim path they concern.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

macro_rules! codes {
    ($($variant:ident => $text:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Code {
            $($variant),*
        }

        impl Code {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),*
                }
            }

            pub const ALL: &'static [Code] = &[$(Code::$variant),*];
        }
    };
}

codes! {
    UnknownSchemaVersion => "unknown_schema_version",
    InvalidSchemaVersion => "invalid_schema_version",
    NestedMachineRoot => "nested_machine_root",
    DuplicateMachineId => "duplicate_machine_id",
    DuplicateArticulationMarker => "duplicate_articulation_marker",
    InvalidEndpoint => "invalid_endpoint",
    EndpointViaAncestor => "endpoint_via_ancestor",
    StaticEndpoint => "static_endpoint",
    BothEndpointsWorld => "both_endpoints_world",
    SameBodyEndpoints => "same_body_endpoints",
    CrossMachineJoint => "cross_machine_joint",
    ExternalJoint => "external_joint",
    UnclassifiedCycle => "unclassified_cycle",
    DisabledJoint => "disabled_joint",
    UnsupportedJointType => "unsupported_joint_type",
    UnsupportedCapability => "unsupported_capability",
    DuplicateName => "duplicate_name",
    MissingControlBody => "missing_control_body",
    InvalidControlBody => "invalid_control_body",
    InvalidControlFrame => "invalid_control_frame",
    InvalidBaseFrame => "invalid_base_frame",
    ConflictingParent => "conflicting_parent",
    DisconnectedBody => "disconnected_body",
    UnanchoredFrame => "unanchored_frame",
    UnsupportedXformOp => "unsupported_xform_op",
    NonStandardDriveToken => "nonstandard_drive_token",
    NonUniformScale => "nonuniform_scale",
    ShearTransform => "shear_transform",
    ReflectionTransform => "reflection_transform",
    InvalidNumeric => "invalid_numeric",
    InvalidMass => "invalid_mass",
    InvalidInertia => "invalid_inertia",
    MissingInertia => "missing_inertia",
    InvalidLimit => "invalid_limit",
    InvalidDrive => "invalid_drive",
    InvalidUnits => "invalid_units",
    InvalidInterface => "invalid_interface",
    InitialStateMismatch => "initial_state_mismatch",
    IsaacArrayLength => "isaac_array_length",
    IsaacUnknownTarget => "isaac_unknown_target",
    IsaacOutsideScope => "isaac_outside_scope",
    UnresolvedControllerJoint => "unresolved_controller_joint",
    InvalidControllerRate => "invalid_controller_rate",
    DuplicateOwner => "duplicate_owner",
    SuggestedRole => "suggested_role",
    LegacyInference => "legacy_inference",
    PayloadUnloaded => "payload_unloaded",
    InstanceProxy => "instance_proxy",
    CompositionError => "composition_error",
    LegacyLinkTree => "legacy_link_tree",
    LegacyDevice => "legacy_device",
    NoMachines => "no_machines",
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub code: Code,
    pub severity: Severity,
    pub path: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn new(code: Code, severity: Severity, path: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            code,
            severity,
            path: path.map(str::to_string),
            message: message.into(),
        }
    }

    pub fn error(code: Code, path: Option<&str>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Error, path, message)
    }

    pub fn warning(code: Code, path: Option<&str>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Warning, path, message)
    }

    pub fn info(code: Code, path: Option<&str>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Info, path, message)
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{} [{}] {}: {}", self.severity.as_str(), self.code.as_str(), path, self.message),
            None => write!(f, "{} [{}] {}", self.severity.as_str(), self.code.as_str(), self.message),
        }
    }
}

/// Sorted for stable manifests: severity first, then code, path and text.
pub fn sort(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(a.code.cmp(&b.code))
            .then(a.path.cmp(&b.path))
            .then(a.message.cmp(&b.message))
    });
}
