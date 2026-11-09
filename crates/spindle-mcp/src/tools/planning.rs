use super::*;

impl ToolRouter {
    pub(super) async fn handle_planning_begin(
        &self,
        input: PlanningBeginInput,
    ) -> anyhow::Result<PlanningBeginOutput> {
        self.service.planning_begin_entry(input).await
    }

    pub(super) async fn handle_planning_advance(
        &self,
        input: PlanningAdvanceInput,
    ) -> anyhow::Result<PlanningAdvanceOutput> {
        self.service.planning_advance_entry(input).await
    }

    pub(super) async fn handle_planning_status(
        &self,
        input: PlanningStatusInput,
    ) -> anyhow::Result<PlanningStatusOutput> {
        self.service.planning_status(input).await
    }

    pub(super) async fn handle_planning_decide(
        &self,
        input: PlanningDecideInput,
    ) -> anyhow::Result<PlanningDecideOutput> {
        self.service.planning_decide_entry(input).await
    }
}
