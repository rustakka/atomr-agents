//! FR-14 — persona clearance ↔ Role `ClearanceContext` validation.
//!
//! A compiled persona declares the clearance level and compartments it
//! must operate at (see [`crate::PersonaMetadata::clearance`] /
//! [`crate::PersonaMetadata::compartments`]). The Role the persona is
//! bound to carries the system-of-record authority as an
//! [`atomr_agents_security::ClearanceContext`]. If those two drift, an
//! analyst could be granted or denied access inconsistently — a
//! material-non-public-information (MNPI) information-wall hazard.
//!
//! [`bind_persona_to_role`] is a **pure, fail-closed** check intended to
//! run at host/compose time (during `OrgProjection` or host bootstrap),
//! *not* on the first money-moving tool call. A mismatch returns a typed
//! [`ClearanceMismatch`] naming the offending compartment or level so
//! startup can abort with an actionable error.

use atomr_agents_security::{ClearanceContext, ClearanceLevel, Compartment};
use thiserror::Error;

/// Why a persona could not be bound to a Role's clearance. Each variant
/// names the specific offending level/compartment so a host bootstrap
/// failure is actionable. Fail-closed: any declared access the Role does
/// not grant is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClearanceMismatch {
    /// The persona declares a clearance level higher than the Role
    /// actually grants.
    #[error("persona requires clearance level {required:?} but role grants only {granted:?}")]
    InsufficientLevel {
        required: ClearanceLevel,
        granted: ClearanceLevel,
    },

    /// The persona declares a compartment the Role's `ClearanceContext`
    /// does not hold.
    #[error("persona requires compartment {0:?} which the role does not grant")]
    MissingCompartment(Compartment),
}

/// Validate, at host/compose time, that a persona's declared clearance
/// and compartments are all granted by the Role it will be bound to.
///
/// Fails closed: returns the first [`ClearanceMismatch`] encountered.
/// A persona that declares *less* access than the Role is allowed (the
/// Role may legitimately be more privileged than this particular
/// persona needs); only persona-declared access the Role lacks is a
/// violation.
///
/// `persona_clearance` of [`ClearanceLevel::Public`] with no compartments
/// always binds successfully against any Role.
pub fn bind_persona_to_role(
    persona_clearance: ClearanceLevel,
    persona_compartments: &[Compartment],
    role: &ClearanceContext,
) -> Result<(), ClearanceMismatch> {
    if persona_clearance > role.level {
        return Err(ClearanceMismatch::InsufficientLevel {
            required: persona_clearance,
            granted: role.level,
        });
    }
    for compartment in persona_compartments {
        if !role.holds(compartment) {
            return Err(ClearanceMismatch::MissingCompartment(compartment.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_security::ClearanceContext;

    fn role() -> ClearanceContext {
        ClearanceContext::new("role:credit-analyst", ClearanceLevel::Restricted)
            .with_compartment("deal:acme")
            .with_compartment("desk:credit")
    }

    #[test]
    fn matching_persona_binds_ok() {
        let r = bind_persona_to_role(
            ClearanceLevel::Confidential,
            &[Compartment::new("deal:acme")],
            &role(),
        );
        assert!(r.is_ok());
    }

    #[test]
    fn equal_level_and_all_compartments_binds_ok() {
        let r = bind_persona_to_role(
            ClearanceLevel::Restricted,
            &[Compartment::new("deal:acme"), Compartment::new("desk:credit")],
            &role(),
        );
        assert!(r.is_ok());
    }

    #[test]
    fn empty_persona_binds_against_any_role() {
        let r = bind_persona_to_role(ClearanceLevel::Public, &[], &role());
        assert!(r.is_ok());
    }

    #[test]
    fn missing_compartment_fails_closed_naming_it() {
        let r = bind_persona_to_role(
            ClearanceLevel::Confidential,
            &[Compartment::new("deal:secret")],
            &role(),
        );
        assert_eq!(
            r,
            Err(ClearanceMismatch::MissingCompartment(Compartment::new("deal:secret")))
        );
    }

    #[test]
    fn higher_level_than_role_fails_closed() {
        let r = bind_persona_to_role(ClearanceLevel::Mnpi, &[], &role());
        assert_eq!(
            r,
            Err(ClearanceMismatch::InsufficientLevel {
                required: ClearanceLevel::Mnpi,
                granted: ClearanceLevel::Restricted,
            })
        );
    }
}
