//! Initial planning staging and human ratification; no creative model dispatch.
use super::*;
use spindle_core::planning::*;
mod apply;

impl SqliteSpindleService {
    pub async fn begin_planning(&self, input: BeginPlanningInput) -> Result<PlanningSession> {
        self.repository.begin_planning(input).await
    }
    pub async fn get_planning_session(&self, id: &str) -> Result<PlanningSession> {
        self.repository.get_planning_session(id).await
    }
    pub async fn stage_planning(&self, input: StagePlanningInput) -> Result<PlanningProposal> {
        self.repository.stage_planning(input).await
    }
    pub async fn list_planning_proposals(
        &self,
        id: &str,
        scope: Option<PlanningScope>,
    ) -> Result<Vec<PlanningProposal>> {
        self.repository.list_planning_proposals(id, scope).await
    }
    pub async fn decide_planning(
        &self,
        input: DecidePlanningInput,
    ) -> Result<Vec<PlanningProposal>> {
        if input.decision == PlanningDecision::Accept {
            let report = self.apply_planning(input).await?;
            if let Some(error) = report.error {
                anyhow::bail!(
                    "planning apply stopped: {error}; inspect planning_apply_status for durable progress"
                );
            }
            return Ok(report.proposals);
        }
        self.repository.decide_planning(input).await
    }
}
#[cfg(test)]
mod tests;
