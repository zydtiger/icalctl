use super::*;

pub(super) fn print_batch(batch: &BatchReport) {
    if batch.dry_run {
        println!("Batch dry run: no Calendar changes were made");
    } else {
        println!("Batch result");
    }
    println!(
        "total={} created={} skipped={} updated={} failed={} not_attempted={} would_create={} would_skip={} would_update={}",
        batch.summary.total,
        batch.summary.created,
        batch.summary.skipped,
        batch.summary.updated,
        batch.summary.failed,
        batch.summary.not_attempted,
        batch.summary.would_create,
        batch.summary.would_skip,
        batch.summary.would_update,
    );
    if !batch.can_write {
        println!("batch is blocked by preflight errors");
    }

    for item in &batch.items {
        let client_id = item
            .client_id
            .as_deref()
            .map(|value| format!(" client_id={value:?}"))
            .unwrap_or_default();
        let event_id = item
            .event_id
            .as_deref()
            .or(item.matched_event_id.as_deref())
            .map(|value| format!(" event_id={value}"))
            .unwrap_or_default();
        let error = item
            .error
            .as_ref()
            .map(|value| format!(" error={:?}", value.message))
            .unwrap_or_default();
        println!(
            "{}. {}{}{}{}",
            item.index + 1,
            item.status,
            client_id,
            event_id,
            error
        );
    }
}
