//! Federation monetary policy (ADR 0009): mint when the treasury cannot pay or is below its floor.

use tracing::info;

use super::TickReport;
use crate::db::{catalog::Federation, history};
use crate::error::Result;
use crate::state::AppState;

pub async fn run(st: &AppState, fed: &Federation, funds_short: bool, report: &mut TickReport) -> Result<()> {
    let balance = st.clients.economie.political_balance(fed.political_entity_id, &fed.currency).await?;
    if !funds_short && balance >= fed.treasury_floor {
        return Ok(());
    }
    let reason = if funds_short { "escrow refused: treasury insufficient" } else { "treasury below floor" };
    st.clients.economie.mint(fed.political_entity_id, fed.mint_batch, &fed.currency, reason).await?;
    history::record_mint(&st.pool, fed.id, fed.mint_batch, reason).await?;
    report.minted += fed.mint_batch;
    info!(federation = %fed.code, amount = fed.mint_batch, balance, reason, "minted");
    Ok(())
}
