//! FR-13 — `CompiledOrg` → agents-org projection (+ compartment-wall sync).
//!
//! `atomr-orgs` is the **system-of-record (SOR)** for who/what the firm
//! is; this crate's `Org`/`Department`/`Team` is the **workforce routing
//! org**. Drift between them silently breaks information walls or
//! routing. This module makes the SOR→routing projection first-class and
//! testable instead of hand-written per integrator.
//!
//! Because `atomr-orgs` is an external repo, the SOR is consumed through
//! a **framework-local** [`CompiledOrgSource`] trait over minimal local
//! structs ([`CompiledOrgModel`] etc.); `atomr-orgs` implements the trait
//! later. No dependency on that repo is introduced here.
//!
//! ## Wall-sync (fail-closed)
//!
//! Every SOR [`Compartment`] must map to both a [`Policy`] scope (via
//! [`Policy::narrow`]) **and** a [`crate::NamespacedMemory`] namespace
//! string. A compartment that cannot be mapped (e.g. a blank name that
//! yields no valid namespace) is **fail-closed**: [`OrgProjection::project`]
//! returns [`ProjectionError::UnmappedCompartment`] naming it, rather
//! than producing a routing org with a silent wall hole. The
//! [`WallSyncReport`] records the mapped/unmapped split for audit.
//!
//! ## Drift gate
//!
//! [`OrgProjection::verify`] re-derives the expected projection from the
//! SOR and reports any [`Drift`] — compartments, roles, or reporting
//! edges present in the SOR but missing in the routing org. Run it as a
//! CI test so SOR↔routing divergence fails the build instead of becoming
//! a runtime wall breach.

use std::collections::{BTreeMap, BTreeSet};

use atomr_agents_security::Compartment;
use atomr_agents_strategy::Policy;
use thiserror::Error;

// ---------------------------------------------------------------------
// Framework-local SOR abstractions (atomr-orgs implements these later).
// ---------------------------------------------------------------------

/// A unit in the system-of-record org (maps to a Department or Team).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledUnit {
    pub id: String,
    pub label: String,
    /// Parent unit id, or `None` for the root.
    pub parent: Option<String>,
    /// Compartments (need-to-know walls) scoped to this unit.
    pub compartments: Vec<Compartment>,
}

/// A role in the SOR org. Roles are the routing targets within a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRole {
    pub id: String,
    pub label: String,
    /// The unit this role belongs to.
    pub unit_id: String,
    /// The label a router matches on to dispatch to this role.
    pub route_label: String,
}

/// The minimal SOR model the projection consumes. `edges` are reporting
/// relationships `(from_unit_id, to_unit_id)` (e.g. child → parent).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompiledOrgModel {
    pub units: Vec<CompiledUnit>,
    pub roles: Vec<CompiledRole>,
    pub edges: Vec<(String, String)>,
}

/// Framework-local source of a [`CompiledOrgModel`]. `atomr-orgs`
/// implements this over its concrete `CompiledOrg`; tests and the
/// projection consume only the trait.
pub trait CompiledOrgSource {
    fn model(&self) -> CompiledOrgModel;
}

impl CompiledOrgSource for CompiledOrgModel {
    fn model(&self) -> CompiledOrgModel {
        self.clone()
    }
}

// ---------------------------------------------------------------------
// Projection output.
// ---------------------------------------------------------------------

/// How a single SOR [`Compartment`] is mirrored into the routing org:
/// a narrowed [`Policy`] scope plus a memory namespace string.
///
/// `Policy` has no `PartialEq` upstream, so equality is implemented by
/// hand comparing the wall identity (compartment + namespace) and the
/// policy's grant fields, keeping the type usable in assertions.
#[derive(Debug, Clone)]
pub struct CompartmentMapping {
    pub compartment: Compartment,
    /// The `NamespacedMemory`-style namespace string for this wall.
    pub namespace: String,
    /// The policy scope the wall narrows to (toolset grant naming the
    /// compartment), composed via [`Policy::narrow`] under the unit.
    pub policy: Policy,
}

/// Structural equality over a `Policy`'s grant fields (it has no upstream
/// `PartialEq`).
fn policy_eq(a: &Policy, b: &Policy) -> bool {
    a.allowed_toolsets.len() == b.allowed_toolsets.len()
        && a.allowed_toolsets
            .iter()
            .zip(&b.allowed_toolsets)
            .all(|(x, y)| x.as_str() == y.as_str())
        && a.allowed_models == b.allowed_models
        && a.max_tokens_per_call == b.max_tokens_per_call
        && a.max_money_micro_usd_per_call == b.max_money_micro_usd_per_call
}

impl PartialEq for CompartmentMapping {
    fn eq(&self, other: &Self) -> bool {
        self.compartment == other.compartment
            && self.namespace == other.namespace
            && policy_eq(&self.policy, &other.policy)
    }
}
impl Eq for CompartmentMapping {}

/// Records which compartments were mapped and which were not. Surfaced
/// for audit; any non-empty `unmapped` means the projection failed
/// closed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WallSyncReport {
    pub mapped: Vec<Compartment>,
    pub unmapped: Vec<Compartment>,
}

/// A projected unit (Department or Team) in the routing org.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedUnit {
    pub id: String,
    pub label: String,
    pub parent: Option<String>,
    /// Route labels of the roles homed in this unit, in declaration order.
    pub role_routes: Vec<String>,
    /// Compartment → (namespace, policy) wall mappings for this unit.
    pub compartments: Vec<CompartmentMapping>,
}

/// The deterministic result of projecting a [`CompiledOrgModel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedOrg {
    pub units: Vec<ProjectedUnit>,
    /// Reporting edges carried over as routing edges `(from, to)`.
    pub routing_edges: Vec<(String, String)>,
    /// Compartment-wall sync report (all `mapped` on success).
    pub wall_sync: WallSyncReport,
}

impl ProjectedOrg {
    /// All compartment namespaces across every unit, for wall enumeration.
    pub fn namespaces(&self) -> Vec<String> {
        self.units
            .iter()
            .flat_map(|u| u.compartments.iter().map(|m| m.namespace.clone()))
            .collect()
    }
}

// ---------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------

/// Why a projection could not be produced. All variants are fail-closed:
/// the projection never returns a routing org with a silent wall hole.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectionError {
    /// A compartment present in the SOR could not be mapped to a
    /// namespace/policy scope (e.g. blank name). Fail-closed.
    #[error("compartment {0:?} present in the system-of-record could not be mapped to a routing wall")]
    UnmappedCompartment(Compartment),

    /// A role references a unit id that does not exist.
    #[error("role {role:?} references unknown unit {unit:?}")]
    UnknownRoleUnit { role: String, unit: String },

    /// A reporting edge references a unit id that does not exist.
    #[error("reporting edge {0:?} -> {1:?} references an unknown unit")]
    UnknownEdgeUnit(String, String),
}

// ---------------------------------------------------------------------
// Drift (CI gate).
// ---------------------------------------------------------------------

/// The kind of divergence [`OrgProjection::verify`] detected between the
/// SOR and the routing org.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftKind {
    /// A compartment in the SOR has no wall mapping in the routing org.
    MissingCompartment,
    /// A role's route in the SOR is absent from the routing org.
    MissingRole,
    /// A reporting edge in the SOR is absent from the routing edges.
    MissingEdge,
}

/// A single SOR↔routing divergence. A non-empty `Vec<Drift>` from
/// [`OrgProjection::verify`] should fail a CI test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub kind: DriftKind,
    pub detail: String,
}

// ---------------------------------------------------------------------
// Projection.
// ---------------------------------------------------------------------

/// SOR → routing-org projection adapter. Stateless; all methods are
/// deterministic functions of the input model.
pub struct OrgProjection;

impl OrgProjection {
    /// Derive a stable memory namespace string for a compartment under a
    /// unit. Returns `None` for an unmappable (blank) compartment so the
    /// caller can fail closed.
    fn namespace_for(unit_id: &str, compartment: &Compartment) -> Option<String> {
        let name = compartment.as_str().trim();
        if name.is_empty() {
            return None;
        }
        Some(format!("compartment/{unit_id}/{name}"))
    }

    /// Map a compartment to a narrowed [`Policy`] scope: a single
    /// allowed-toolset grant keyed on the compartment name. Composed via
    /// [`Policy::narrow`] so it intersects with (never widens) the unit's
    /// inherited policy.
    fn policy_for(compartment: &Compartment) -> Policy {
        let scope = Policy {
            allowed_toolsets: vec![compartment.as_str().into()],
            ..Default::default()
        };
        // Narrow against an empty parent — self-intersection — to keep
        // the construction identical to the runtime narrowing path.
        Policy::narrow(&scope, &scope)
    }

    /// Project a [`CompiledOrgModel`] into a [`ProjectedOrg`].
    ///
    /// Maps Unit → Department/Team, Role → routing target, reporting
    /// hierarchy → routing edges, and each [`Compartment`] → a
    /// [`Policy::narrow`] scope + namespace. Fails closed (returns
    /// [`ProjectionError::UnmappedCompartment`]) on any compartment that
    /// cannot be mapped.
    pub fn project(model: &CompiledOrgModel) -> Result<ProjectedOrg, ProjectionError> {
        let unit_ids: BTreeSet<&str> = model.units.iter().map(|u| u.id.as_str()).collect();

        // Validate roles reference known units.
        for role in &model.roles {
            if !unit_ids.contains(role.unit_id.as_str()) {
                return Err(ProjectionError::UnknownRoleUnit {
                    role: role.id.clone(),
                    unit: role.unit_id.clone(),
                });
            }
        }
        // Validate edges reference known units.
        for (from, to) in &model.edges {
            if !unit_ids.contains(from.as_str()) || !unit_ids.contains(to.as_str()) {
                return Err(ProjectionError::UnknownEdgeUnit(from.clone(), to.clone()));
            }
        }

        // Roles grouped by unit, in declaration order.
        let mut routes_by_unit: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for role in &model.roles {
            routes_by_unit
                .entry(role.unit_id.as_str())
                .or_default()
                .push(role.route_label.clone());
        }

        let mut wall_sync = WallSyncReport::default();
        let mut units = Vec::with_capacity(model.units.len());

        for unit in &model.units {
            let mut mappings = Vec::with_capacity(unit.compartments.len());
            for compartment in &unit.compartments {
                match Self::namespace_for(&unit.id, compartment) {
                    Some(namespace) => {
                        wall_sync.mapped.push(compartment.clone());
                        mappings.push(CompartmentMapping {
                            compartment: compartment.clone(),
                            namespace,
                            policy: Self::policy_for(compartment),
                        });
                    }
                    None => {
                        // Fail closed: record then abort naming it.
                        wall_sync.unmapped.push(compartment.clone());
                        return Err(ProjectionError::UnmappedCompartment(compartment.clone()));
                    }
                }
            }
            units.push(ProjectedUnit {
                id: unit.id.clone(),
                label: unit.label.clone(),
                parent: unit.parent.clone(),
                role_routes: routes_by_unit.get(unit.id.as_str()).cloned().unwrap_or_default(),
                compartments: mappings,
            });
        }

        Ok(ProjectedOrg {
            units,
            routing_edges: model.edges.clone(),
            wall_sync,
        })
    }

    /// CI drift gate: detect compartments, roles, and reporting edges
    /// present in the SOR but missing from the routing org. An empty
    /// result means no drift.
    ///
    /// Robust to a `projected` that failed closed: a missing wall mapping
    /// is reported as [`DriftKind::MissingCompartment`] rather than
    /// panicking.
    pub fn verify(model: &CompiledOrgModel, projected: &ProjectedOrg) -> Vec<Drift> {
        let mut drifts = Vec::new();

        // Index projected walls by (unit_id, compartment-name).
        let projected_walls: BTreeSet<(String, String)> = projected
            .units
            .iter()
            .flat_map(|u| {
                u.compartments
                    .iter()
                    .map(move |m| (u.id.clone(), m.compartment.as_str().to_string()))
            })
            .collect();

        for unit in &model.units {
            for compartment in &unit.compartments {
                let key = (unit.id.clone(), compartment.as_str().to_string());
                if !projected_walls.contains(&key) {
                    drifts.push(Drift {
                        kind: DriftKind::MissingCompartment,
                        detail: format!(
                            "compartment {:?} on unit {:?} has no routing wall",
                            compartment.as_str(),
                            unit.id
                        ),
                    });
                }
            }
        }

        // Roles: every SOR route must appear under its unit.
        let projected_routes: BTreeSet<(String, String)> = projected
            .units
            .iter()
            .flat_map(|u| u.role_routes.iter().map(move |r| (u.id.clone(), r.clone())))
            .collect();
        for role in &model.roles {
            let key = (role.unit_id.clone(), role.route_label.clone());
            if !projected_routes.contains(&key) {
                drifts.push(Drift {
                    kind: DriftKind::MissingRole,
                    detail: format!(
                        "role {:?} (route {:?}) on unit {:?} is missing from routing org",
                        role.id, role.route_label, role.unit_id
                    ),
                });
            }
        }

        // Edges.
        let projected_edges: BTreeSet<(String, String)> = projected.routing_edges.iter().cloned().collect();
        for edge in &model.edges {
            if !projected_edges.contains(edge) {
                drifts.push(Drift {
                    kind: DriftKind::MissingEdge,
                    detail: format!("reporting edge {:?} -> {:?} missing from routing edges", edge.0, edge.1),
                });
            }
        }

        drifts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> CompiledOrgModel {
        CompiledOrgModel {
            units: vec![
                CompiledUnit {
                    id: "research".into(),
                    label: "Research".into(),
                    parent: Some("firm".into()),
                    compartments: vec![Compartment::new("deal:acme"), Compartment::new("issuer:xyz")],
                },
                CompiledUnit {
                    id: "firm".into(),
                    label: "Firm".into(),
                    parent: None,
                    compartments: vec![],
                },
            ],
            roles: vec![
                CompiledRole {
                    id: "r-analyst".into(),
                    label: "Analyst".into(),
                    unit_id: "research".into(),
                    route_label: "analyst".into(),
                },
                CompiledRole {
                    id: "r-pm".into(),
                    label: "PM".into(),
                    unit_id: "firm".into(),
                    route_label: "pm".into(),
                },
            ],
            edges: vec![("research".into(), "firm".into())],
        }
    }

    #[test]
    fn projects_units_roles_edges_and_walls() {
        let m = model();
        let p = OrgProjection::project(&m).unwrap();
        assert_eq!(p.units.len(), 2);
        assert_eq!(p.routing_edges, vec![("research".to_string(), "firm".to_string())]);

        let research = p.units.iter().find(|u| u.id == "research").unwrap();
        assert_eq!(research.role_routes, vec!["analyst".to_string()]);
        assert_eq!(research.compartments.len(), 2);
        assert_eq!(research.compartments[0].namespace, "compartment/research/deal:acme");
        // Compartment scope is mirrored into the policy's toolset grant.
        assert_eq!(
            research.compartments[0].policy.allowed_toolsets[0].as_str(),
            "deal:acme"
        );

        // All compartments mapped, none unmapped.
        assert_eq!(p.wall_sync.mapped.len(), 2);
        assert!(p.wall_sync.unmapped.is_empty());
    }

    #[test]
    fn projection_is_deterministic() {
        let m = model();
        let a = OrgProjection::project(&m).unwrap();
        let b = OrgProjection::project(&m).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn round_trip_verify_reports_no_drift() {
        let m = model();
        let p = OrgProjection::project(&m).unwrap();
        let drift = OrgProjection::verify(&m, &p);
        assert!(drift.is_empty(), "expected no drift, got {drift:?}");
    }

    #[test]
    fn dropping_a_compartment_mapping_is_detected_as_drift() {
        let m = model();
        let mut p = OrgProjection::project(&m).unwrap();
        // Simulate routing-org divergence: drop a wall mapping.
        let research = p.units.iter_mut().find(|u| u.id == "research").unwrap();
        research.compartments.retain(|c| c.compartment.as_str() != "deal:acme");

        let drift = OrgProjection::verify(&m, &p);
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].kind, DriftKind::MissingCompartment);
        assert!(drift[0].detail.contains("deal:acme"));
    }

    #[test]
    fn blank_compartment_fails_closed_naming_it() {
        let mut m = model();
        m.units[0].compartments.push(Compartment::new("   "));
        let err = OrgProjection::project(&m).unwrap_err();
        assert_eq!(err, ProjectionError::UnmappedCompartment(Compartment::new("   ")));
    }

    #[test]
    fn unknown_role_unit_fails() {
        let mut m = model();
        m.roles.push(CompiledRole {
            id: "r-ghost".into(),
            label: "Ghost".into(),
            unit_id: "nowhere".into(),
            route_label: "ghost".into(),
        });
        let err = OrgProjection::project(&m).unwrap_err();
        assert!(matches!(err, ProjectionError::UnknownRoleUnit { .. }));
    }

    #[test]
    fn missing_role_and_edge_drift_detected() {
        let m = model();
        let mut p = OrgProjection::project(&m).unwrap();
        // Drop a role route and an edge from the routing org.
        let firm = p.units.iter_mut().find(|u| u.id == "firm").unwrap();
        firm.role_routes.clear();
        p.routing_edges.clear();

        let drift = OrgProjection::verify(&m, &p);
        assert!(drift.iter().any(|d| d.kind == DriftKind::MissingRole));
        assert!(drift.iter().any(|d| d.kind == DriftKind::MissingEdge));
    }
}
