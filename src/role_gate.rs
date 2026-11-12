//! 角色能力检查（S9 / S17 / S19）。
//!
//! **所有写方法与断言签名必须在校验后端逻辑之前调用本模块**：
//!
//! - `create_anchor` / `revoke_anchor` / `renew_anchor` → [`RoleGate::ensure_issuer`]
//! - `update_authorized_domains` / `update_controller_did` → [`RoleGate::ensure_self`]
//! - `sign_participant_assertion` → [`RoleGate::ensure_signer`]
//!
//! 本模块**不读取** `business_types`（S20）：权限只取决于 [`SdkRole`] 与
//! 自身 `participant_id`。

use crate::{ChainSdkError, SdkRole};

/// 角色门禁：由三个后端实现，提供统一的越权判定。
pub(crate) trait RoleGate {
    /// 本 SDK 实例的配置角色。
    fn role(&self) -> SdkRole;

    /// 本实例所属的 `participant_id`（自身管理写的范围）。
    fn self_participant_id(&self) -> u32;

    /// 写方法门禁：Issuer 放行；Verifier 返回 [`ChainSdkError::Unauthorized`]。
    fn ensure_issuer(&self) -> Result<(), ChainSdkError> {
        match self.role() {
            SdkRole::Issuer => Ok(()),
            SdkRole::Verifier => Err(ChainSdkError::Unauthorized(
                "role Verifier cannot perform write operation".into(),
            )),
        }
    }

    /// 自身管理写门禁：先要求 Issuer，再要求 `participant_id` 等于自身（S19）。
    fn ensure_self(&self, participant_id: u32) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        if participant_id != self.self_participant_id() {
            return Err(ChainSdkError::Unauthorized(format!(
                "participant {} cannot manage participant {}; only chain-owner-sdk can",
                self.self_participant_id(),
                participant_id
            )));
        }
        Ok(())
    }

    /// 断言签名门禁：Issuer 放行；Verifier 返回
    /// [`ChainSdkError::SignerUnavailable`]（S9 / S17）。
    fn ensure_signer(&self) -> Result<(), ChainSdkError> {
        match self.role() {
            SdkRole::Issuer => Ok(()),
            SdkRole::Verifier => Err(ChainSdkError::SignerUnavailable(
                "role Verifier has no participant-did signer".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::RoleGate;
    use crate::{ChainSdkError, SdkRole};

    struct Dummy {
        role: SdkRole,
        participant_id: u32,
    }

    impl RoleGate for Dummy {
        fn role(&self) -> SdkRole {
            self.role
        }

        fn self_participant_id(&self) -> u32 {
            self.participant_id
        }
    }

    #[test]
    fn verifier_cannot_write_or_sign() {
        let gate = Dummy {
            role: SdkRole::Verifier,
            participant_id: 4,
        };
        assert!(matches!(
            gate.ensure_issuer(),
            Err(ChainSdkError::Unauthorized(_))
        ));
        assert!(matches!(
            gate.ensure_self(4),
            Err(ChainSdkError::Unauthorized(_))
        ));
        assert!(matches!(
            gate.ensure_signer(),
            Err(ChainSdkError::SignerUnavailable(_))
        ));
    }

    #[test]
    fn issuer_can_write_self_but_not_others() {
        let gate = Dummy {
            role: SdkRole::Issuer,
            participant_id: 4,
        };
        assert!(gate.ensure_issuer().is_ok());
        assert!(gate.ensure_self(4).is_ok());
        assert!(gate.ensure_signer().is_ok());
        assert!(matches!(
            gate.ensure_self(999),
            Err(ChainSdkError::Unauthorized(_))
        ));
    }
}
