//! Initial planning service seam. No model calls or domain apply in Phase A.
use super::*;
use spindle_core::planning::*;

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
        self.repository.decide_planning(input).await
    }
}
#[cfg(test)]
mod tests;
