use super::{
    account_auth_error, history_store, load_history_summary, now_ms, update_sync_status,
    write_account_log, Registry, SyncError, SyncRuntime, HISTORY_WINDOW_SIZE,
};
use splat3_widget_core::history::{
    HistoryFetchProgress, HistoryFetchedRecord, HistoryProgressCallback, HistoryRecordCallback,
    HistorySyncSummary, SplatNetDataClient, SplatNetDataConfig,
};
use splat3_widget_core::SecretStore;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn mark_not_ready(registry: &Arc<Mutex<Registry>>, account_id: &str) -> Result<(), SyncError> {
    registry
        .lock()
        .map_err(|_| "account registry is locked")?
        .set_splatnet_ready(account_id, false)
        .map_err(SyncError::Other)
}

pub(super) async fn sync_account_inner(
    account_id: &str,
    registry: &Arc<Mutex<Registry>>,
    history_dir: &Path,
    auth_log_path: &Path,
    sync_runtime: &Arc<SyncRuntime>,
) -> Result<HistorySyncSummary, SyncError> {
    let (metadata, secret_store) = {
        let registry = registry.lock().map_err(|_| "account registry is locked")?;
        let metadata = registry
            .accounts()
            .iter()
            .find(|account| account.account_id == account_id)
            .cloned()
            .ok_or_else(|| format!("unknown account: {account_id}"))?;
        if registry.is_account_deletion_pending(account_id) {
            return Err(SyncError::Other(
                "该账号正在完成删除，请稍后重试".to_owned(),
            ));
        }
        (metadata, registry.secret_store())
    };
    // Read the Keychain outside the registry lock: it may wait on a macOS
    // authorization prompt. The account cannot be deleted while it syncs.
    let secret = match secret_store.get(account_id) {
        Ok(Some(secret)) => secret,
        Ok(None) => {
            mark_not_ready(registry, account_id)?;
            write_account_log(
                auth_log_path,
                "credentials",
                &metadata.display_name,
                account_id,
                "系统凭据库里没有这个账号的登录信息，需要重新获取权限",
            );
            return Err(SyncError::Other("该账号没有保存登录信息".to_owned()));
        }
        Err(error) => {
            mark_not_ready(registry, account_id)?;
            write_account_log(
                auth_log_path,
                "credentials",
                &metadata.display_name,
                account_id,
                &format!("读取系统凭据库中的登录信息失败，需要重新获取权限：{error}"),
            );
            return Err(SyncError::Other(error));
        }
    };
    let game_web_token = secret
        .game_web_token
        .ok_or_else(|| "请先获取 SplatNet 权限，再同步历史记录".to_owned())?;
    let bullet_token = secret
        .bullet_token
        .ok_or_else(|| "请先获取 SplatNet 权限，再同步历史记录".to_owned())?;
    let mut config = SplatNetDataConfig::new(
        game_web_token,
        bullet_token,
        metadata.language.as_deref().unwrap_or("en-US"),
        metadata.country.as_deref().unwrap_or("US"),
    );
    config.web_view_version = sync_runtime.web_view_version();

    update_sync_status(sync_runtime, account_id, |status| {
        status.state = "preparing".to_owned();
        status.account_id = Some(account_id.to_owned());
        status.message = "准备同步…".to_owned();
        status.phase = "preparing".to_owned();
        status.progress_completed = 0;
        status.progress_total = 0;
        status.retry_attempt = 0;
        status.started_at_ms = Some(now_ms());
        status.completed_at_ms = None;
        status.error = None;
    });

    let existing_summary = load_history_summary(history_dir, account_id).map_err(|error| {
        account_auth_error(
            auth_log_path,
            "history-summary",
            &metadata.display_name,
            account_id,
            error,
        )
    })?;
    let initial_sync =
        history_store::needs_full_sync(history_dir, account_id).map_err(|error| {
            account_auth_error(
                auth_log_path,
                "history-state",
                &metadata.display_name,
                account_id,
                error,
            )
        })?;
    let known_record_limit = (!initial_sync).then_some(HISTORY_WINDOW_SIZE);
    let (known_battle_ids, known_coop_ids) =
        history_store::load_known_record_ids(history_dir, account_id, known_record_limit).map_err(
            |error| {
                account_auth_error(
                    auth_log_path,
                    "history-dedup",
                    &metadata.display_name,
                    account_id,
                    error,
                )
            },
        )?;

    let sync_description = if initial_sync {
        "正在拉取历史数据…"
    } else {
        "检查新数据…"
    };
    update_sync_status(sync_runtime, account_id, |status| {
        status.state = "syncing".to_owned();
        status.account_id = Some(account_id.to_owned());
        status.message = sync_description.to_owned();
        status.phase = "syncing".to_owned();
        status.progress_completed = 0;
        status.progress_total = 0;
        status.retry_attempt = 0;
        status.initial = initial_sync;
        status.battle_count = existing_summary
            .as_ref()
            .map(|summary| summary.battle_count)
            .unwrap_or_default();
        status.coop_count = existing_summary
            .as_ref()
            .map(|summary| summary.coop_count)
            .unwrap_or_default();
    });
    write_account_log(
        auth_log_path,
        "history-sync",
        &metadata.display_name,
        account_id,
        if initial_sync {
            "开始首次完整同步：拉取当前账号可用的全部对战和打工记录"
        } else {
            "开始增量同步：检查最近 50 条对战和 50 条打工记录"
        },
    );

    let fetched_at_ms = now_ms();
    history_store::mark_sync_started(history_dir, account_id, fetched_at_ms).map_err(|error| {
        account_auth_error(
            auth_log_path,
            "history-state",
            &metadata.display_name,
            account_id,
            error,
        )
    })?;

    let client = SplatNetDataClient::connect(config).await.map_err(|error| {
        account_auth_error(
            auth_log_path,
            "history-client",
            &metadata.display_name,
            account_id,
            error,
        )
    })?;
    if let Some(version) = client.web_view_version() {
        sync_runtime.set_web_view_version(Some(version));
    }
    let progress_status = Arc::clone(sync_runtime);
    let progress_account_id = account_id.to_owned();
    let unavailable_detail_count = Arc::new(AtomicUsize::new(0));
    let unavailable_detail_count_for_progress = Arc::clone(&unavailable_detail_count);
    let progress: HistoryProgressCallback = Arc::new(move |progress: HistoryFetchProgress| {
        if progress.phase == "partial-details" {
            unavailable_detail_count_for_progress.store(progress.completed, Ordering::Relaxed);
        }
        update_sync_status(&progress_status, &progress_account_id, |status| {
            status.phase = progress.phase.clone();
            status.progress_completed = progress.completed;
            status.progress_total = progress.total;
            status.retry_attempt = progress.retry_attempt;
            status.message = match progress.phase.as_str() {
                "battle-list" => "正在获取最近对战列表…".to_owned(),
                "x-measurements" => "正在检查 X 赛结算分数…".to_owned(),
                "coop-list" => "正在获取最近打工列表…".to_owned(),
                "battle-details" => "正在获取对战详情…".to_owned(),
                "coop-details" => "正在获取打工详情…".to_owned(),
                "partial-details" => {
                    format!("{} 条详情暂不可读，正在完成同步…", progress.completed)
                }
                phase if phase.starts_with("retrying:") => "网络波动，正在重试…".to_owned(),
                _ => "正在同步数据…".to_owned(),
            };
        });
    });
    let record_writer = Arc::new(Mutex::new(
        history_store::HistoryRecordWriter::open(history_dir).map_err(|error| {
            account_auth_error(
                auth_log_path,
                "history-store",
                &metadata.display_name,
                account_id,
                error,
            )
        })?,
    ));
    let record_writer_for_callback = Arc::clone(&record_writer);
    let record_account_id = account_id.to_owned();
    let x_settlement_updates = Arc::new(AtomicUsize::new(0));
    let x_settlement_updates_for_callback = Arc::clone(&x_settlement_updates);
    let rotation_updates = Arc::new(AtomicUsize::new(0));
    let rotation_updates_for_callback = Arc::clone(&rotation_updates);
    let on_record: HistoryRecordCallback = Arc::new(move |record: HistoryFetchedRecord| {
        let is_x_settlement = matches!(&record, HistoryFetchedRecord::XBattleSettlement { .. });
        let changed = record_writer_for_callback
            .lock()
            .map_err(|_| "历史记录写入器已锁定".to_owned())?
            .save(&record_account_id, fetched_at_ms, record)?;
        if changed {
            let updates = if is_x_settlement {
                &x_settlement_updates_for_callback
            } else {
                &rotation_updates_for_callback
            };
            updates.fetch_add(1, Ordering::Relaxed);
        }
        Ok(changed)
    });
    // 查询失败时也要存回去：先记下的新 X 比赛，下次同步接着查。
    let mut x_check = sync_runtime.x_settlement_check(account_id);
    let fetched = if initial_sync {
        client
            .fetch_history_full_with_progress_and_records_from_typed(
                &known_battle_ids,
                &known_coop_ids,
                fetched_at_ms,
                Arc::clone(&progress),
                Some(Arc::clone(&on_record)),
                &mut x_check,
            )
            .await
    } else {
        client
            .fetch_history_incremental_with_progress_and_records_typed(
                HISTORY_WINDOW_SIZE,
                HISTORY_WINDOW_SIZE,
                &known_battle_ids,
                &known_coop_ids,
                fetched_at_ms,
                Arc::clone(&progress),
                Some(Arc::clone(&on_record)),
                &mut x_check,
            )
            .await
    };
    sync_runtime.set_x_settlement_check(account_id, x_check);
    let cache = fetched.map_err(|error| {
        // 网页版本号过期时 SplatNet 会拒绝请求，下次同步重新查。
        sync_runtime.set_web_view_version(None);
        let kind = error.kind();
        let message = account_auth_error(
            auth_log_path,
            "history-fetch",
            &metadata.display_name,
            account_id,
            error.to_string(),
        );
        SyncError::from_history(kind, message)
    })?;
    drop(on_record);
    drop(record_writer);
    let added_battle_count = cache.battles.len();
    let added_coop_count = cache.coop.len();
    let unavailable_detail_count = unavailable_detail_count.load(Ordering::Relaxed);
    let x_settlement_update_count = x_settlement_updates.load(Ordering::Relaxed);
    let rotation_update_count = rotation_updates.load(Ordering::Relaxed);
    if unavailable_detail_count > 0
        && (initial_sync || added_battle_count > 0 || added_coop_count > 0)
    {
        write_account_log(
            auth_log_path,
            "history-fetch",
            &metadata.display_name,
            account_id,
            &format!(
                "{} 条战绩详情暂不可读取，其余记录已保存；后续同步会继续重试",
                unavailable_detail_count
            ),
        );
    }
    if x_settlement_update_count > 0 {
        write_account_log(
            auth_log_path,
            "history-sync",
            &metadata.display_name,
            account_id,
            &format!(
                "X 赛测定已结算，更新卡片分数 {} 条",
                x_settlement_update_count
            ),
        );
    }
    if initial_sync || added_battle_count > 0 || x_settlement_update_count > 0 {
        history_store::rebuild_overlay_snapshot(history_dir, account_id).map_err(|error| {
            account_auth_error(
                auth_log_path,
                "overlay-cache",
                &metadata.display_name,
                account_id,
                error,
            )
        })?;
    }
    history_store::mark_sync_complete(history_dir, account_id, fetched_at_ms).map_err(|error| {
        account_auth_error(
            auth_log_path,
            "history-state",
            &metadata.display_name,
            account_id,
            error,
        )
    })?;
    let mut summary = load_history_summary(history_dir, account_id)
        .map_err(|error| {
            account_auth_error(
                auth_log_path,
                "history-summary",
                &metadata.display_name,
                account_id,
                error,
            )
        })?
        .unwrap_or_default();
    summary.added_battle_count = added_battle_count;
    summary.added_coop_count = added_coop_count;
    summary.records_updated = x_settlement_update_count > 0 || rotation_update_count > 0;
    write_account_log(
        auth_log_path,
        "history-sync",
        &metadata.display_name,
        account_id,
        &format!(
            "历史记录同步成功：新增 {} 条对战、{} 条打工；归档共 {} 条对战、{} 条打工",
            summary.added_battle_count,
            summary.added_coop_count,
            summary.battle_count,
            summary.coop_count
        ),
    );
    Ok(summary)
}
