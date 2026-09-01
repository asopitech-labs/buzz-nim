//! Durable write-before-effect adapter driven by Nim ledger policy.

use std::{sync::Arc, time::Duration};

use nimino_boundary::{
    BoundaryClient, BoundaryRequest, BoundaryResult, CallContext, EffectLedgerDecision,
    EffectLedgerEffect, EffectLedgerError, EffectLedgerPlan, EffectLedgerPortEffect,
    EffectLedgerState, EffectLedgerStatus, EffectPolicyRequest, EffectPolicyResult, EffectReceipt,
    EffectReceiptOutcome,
};
use nimino_control::LeaseClient;
use nimino_core::tenant::CommunityId;
use nimino_store::{CanonicalCommit, NodeStorePort, RecordClass, RecordWrite, StoreError};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::WorkflowError;

const RECORD_TYPE: &str = "workflow_effect";
const LEASE_TICKS: u64 = 600_000;
const MAX_STORE_RETRIES: usize = 8;

/// Immutable identity and resolved content of one external workflow effect.
pub(crate) struct EffectIdentity<'a> {
    pub(crate) community_id: CommunityId,
    pub(crate) workflow_id: Uuid,
    pub(crate) run_id: Uuid,
    pub(crate) step_id: &'a str,
    pub(crate) resolved_effect: &'a crate::ActionDef,
}

/// Token proving that the execution marker was durable before external I/O.
pub(crate) struct EffectPermit {
    key: String,
    state: EffectLedgerState,
}

/// Production adapter that executes only the persistence decisions returned by Nim.
pub(crate) struct EffectLedgerAdapter {
    boundary: BoundaryClient,
    store: Arc<dyn NodeStorePort>,
    lease: LeaseClient,
}

impl EffectLedgerAdapter {
    pub(crate) fn new(
        boundary: BoundaryClient,
        store: Arc<dyn NodeStorePort>,
        lease: LeaseClient,
    ) -> Self {
        Self {
            boundary,
            store,
            lease,
        }
    }

    pub(crate) async fn prepare(
        &self,
        identity: EffectIdentity<'_>,
    ) -> Result<EffectPermit, WorkflowError> {
        let effect_bytes = serde_json::to_vec(identity.resolved_effect)
            .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
        let effect_digest = hex::encode(Sha256::digest(&effect_bytes));
        let community_id = identity.community_id.to_string();
        let key = effect_key(
            &community_id,
            &identity.workflow_id.to_string(),
            &identity.run_id.to_string(),
            identity.step_id,
            &effect_digest,
        );
        let lease_resource_id = format!("workflow-effect/{key}");
        let mut state = self
            .load(&community_id, &key)?
            .unwrap_or_else(|| EffectLedgerState {
                valid: true,
                community_id: community_id.clone(),
                workflow_id: identity.workflow_id.to_string(),
                run_id: identity.run_id.to_string(),
                step_id: identity.step_id.to_owned(),
                idempotency_key: key.clone(),
                effect_digest,
                lease_resource_id: lease_resource_id.clone(),
                revision: 0,
                attempt: 0,
                status: EffectLedgerStatus::Pending,
                owner_node_id: String::new(),
                fence_token: 0,
                receipt: None,
                reconciled_by: String::new(),
                reconcile_reason: String::new(),
            });

        if matches!(
            state.status,
            EffectLedgerStatus::Claimed | EffectLedgerStatus::Executing
        ) {
            let (lease_state, fact) = self
                .lease
                .policy_context(&lease_resource_id)
                .await
                .map_err(workflow_error)?;
            state = self
                .apply_plan(
                    &community_id,
                    &key,
                    EffectPolicyRequest::Recover {
                        state,
                        lease_state,
                        fact,
                    },
                )
                .await?
                .state;
        }
        if state.status == EffectLedgerStatus::Unknown {
            return Err(WorkflowError::WebhookError(
                "workflow effect outcome is unknown; manual reconciliation is required".into(),
            ));
        }

        let owner = self.lease.local_node_id();
        let transition_id = format!("{key}:{}", state.attempt.saturating_add(1));
        let active = self
            .lease
            .grant(
                lease_resource_id.clone(),
                transition_id,
                vec![owner.clone()],
                LEASE_TICKS,
            )
            .await
            .map_err(workflow_error)?;
        let (lease_state, fact) = self
            .lease
            .policy_context(&lease_resource_id)
            .await
            .map_err(workflow_error)?;
        state = self
            .apply_plan(
                &community_id,
                &key,
                EffectPolicyRequest::Claim {
                    state,
                    owner_node_id: owner.clone(),
                    fence_token: active.fence_token,
                    lease_state,
                    fact,
                },
            )
            .await?
            .state;

        let (lease_state, fact) = self
            .lease
            .policy_context(&lease_resource_id)
            .await
            .map_err(workflow_error)?;
        let decision = self
            .apply_plan(
                &community_id,
                &key,
                EffectPolicyRequest::Execute {
                    state,
                    owner_node_id: owner,
                    fence_token: active.fence_token,
                    lease_state,
                    fact,
                },
            )
            .await?;
        if decision.effect != EffectLedgerEffect::ExecuteExternal {
            return Err(WorkflowError::WebhookError(
                "workflow effect execution marker was not accepted".into(),
            ));
        }
        Ok(EffectPermit {
            key,
            state: decision.state,
        })
    }

    pub(crate) async fn record<T: std::fmt::Debug>(
        &self,
        permit: EffectPermit,
        result: &Result<T, WorkflowError>,
    ) -> Result<(), WorkflowError> {
        let (outcome, result_text) = match result {
            Ok(value) => (EffectReceiptOutcome::Succeeded, format!("{value:?}")),
            Err(error) => (
                EffectReceiptOutcome::Failed,
                format!("{}\0{error}", error.code()),
            ),
        };
        let result_digest = hex::encode(Sha256::digest(result_text.as_bytes()));
        let receipt = EffectReceipt {
            outcome,
            receipt_id: format!("{}:{result_digest}", permit.key),
            result_digest,
        };
        let community_id = permit.state.community_id.clone();
        let owner_node_id = permit.state.owner_node_id.clone();
        let fence_token = permit.state.fence_token;
        let decision = self
            .apply_plan(
                &community_id,
                &permit.key,
                EffectPolicyRequest::Receipt {
                    state: permit.state,
                    owner_node_id,
                    fence_token,
                    receipt,
                },
            )
            .await?;
        if decision.effect != EffectLedgerEffect::ReceiptRecorded
            && decision.effect != EffectLedgerEffect::Replay
        {
            return Err(WorkflowError::WebhookError(
                "workflow effect receipt was not accepted".into(),
            ));
        }
        Ok(())
    }

    fn load(
        &self,
        community_id: &str,
        key: &str,
    ) -> Result<Option<EffectLedgerState>, WorkflowError> {
        self.store
            .get(RecordClass::Canonical, community_id, RECORD_TYPE, key)
            .map_err(workflow_error)?
            .map(|record| serde_json::from_value(record.value).map_err(workflow_error))
            .transpose()
    }

    async fn apply_plan(
        &self,
        community_id: &str,
        key: &str,
        request: EffectPolicyRequest,
    ) -> Result<EffectLedgerDecision, WorkflowError> {
        let plan = match self.call(request).await? {
            EffectPolicyResult::Claim { result }
            | EffectPolicyResult::Execute { result }
            | EffectPolicyResult::Receipt { result }
            | EffectPolicyResult::Recover { result }
            | EffectPolicyResult::Reconcile { result } => result,
            EffectPolicyResult::Settle { .. } => {
                return Err(WorkflowError::InvalidDefinition(
                    "Nim returned an unexpected effect plan".into(),
                ));
            }
        };
        if plan.error != EffectLedgerError::None {
            return self.settle(plan, false).await.and_then(reject_decision);
        }
        let persisted = if plan.port_effect == EffectLedgerPortEffect::CommitCanonical {
            self.persist(community_id, key, &plan).is_ok()
        } else {
            true
        };
        let decision = self.settle(plan, persisted).await?;
        if decision.error != EffectLedgerError::None {
            return reject_decision(decision);
        }
        Ok(decision)
    }

    fn persist(
        &self,
        community_id: &str,
        key: &str,
        plan: &EffectLedgerPlan,
    ) -> Result<(), StoreError> {
        for _ in 0..MAX_STORE_RETRIES {
            let current = self
                .store
                .get(RecordClass::Canonical, community_id, RECORD_TYPE, key)?;
            let matches_before = match current {
                Some(record) => {
                    serde_json::from_value::<EffectLedgerState>(record.value)? == plan.before_state
                }
                None => plan.before_state.revision == 0,
            };
            if !matches_before {
                return Err(StoreError::IntentConflict);
            }
            let checkpoint = self.store.canonical_checkpoint(community_id)?;
            let commit = CanonicalCommit {
                intent_id: format!("effect:{key}:{}", plan.next_state.revision),
                community_id: community_id.to_owned(),
                expected_checkpoint: checkpoint,
                writes: vec![RecordWrite {
                    record_type: RECORD_TYPE.to_owned(),
                    key: key.to_owned(),
                    deleted: false,
                    value: serde_json::to_value(&plan.next_state)?,
                }],
            };
            match self.store.commit_canonical(commit) {
                Ok(_) => return Ok(()),
                Err(StoreError::CheckpointConflict { .. }) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(StoreError::InvalidInput(
            "effect ledger checkpoint retry limit exceeded",
        ))
    }

    async fn settle(
        &self,
        plan: EffectLedgerPlan,
        persistence_succeeded: bool,
    ) -> Result<EffectLedgerDecision, WorkflowError> {
        match self
            .call(EffectPolicyRequest::Settle {
                plan,
                persistence_succeeded,
            })
            .await?
        {
            EffectPolicyResult::Settle { result } => Ok(result),
            _ => Err(WorkflowError::InvalidDefinition(
                "Nim returned an unexpected effect settlement".into(),
            )),
        }
    }

    async fn call(
        &self,
        request: EffectPolicyRequest,
    ) -> Result<EffectPolicyResult, WorkflowError> {
        match self
            .boundary
            .call(
                BoundaryRequest::effect_policy(request),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .map_err(workflow_error)?
        {
            BoundaryResult::EffectPolicy(result) => Ok(result),
            _ => Err(WorkflowError::InvalidDefinition(
                "Nim returned an unexpected effect result".into(),
            )),
        }
    }
}

fn reject_decision<T>(decision: EffectLedgerDecision) -> Result<T, WorkflowError> {
    Err(WorkflowError::WebhookError(format!(
        "workflow effect rejected ({:?}, lease {:?})",
        decision.error, decision.lease_error
    )))
}

fn workflow_error(error: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::WebhookError(error.to_string())
}

fn effect_key(
    community_id: &str,
    workflow_id: &str,
    run_id: &str,
    step_id: &str,
    effect_digest: &str,
) -> String {
    let mut hasher = Sha256::new();
    for component in [community_id, workflow_id, run_id, step_id, effect_digest] {
        hasher.update((component.len() as u64).to_be_bytes());
        hasher.update(component.as_bytes());
    }
    hex::encode(hasher.finalize())
}
