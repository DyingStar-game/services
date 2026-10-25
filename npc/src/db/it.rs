//! Database integration tests (see `testdb`).

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::future::join_all;
use serde_json::json;
use uuid::Uuid;

use super::{events, npcs, patterns, tasks, testdb, workplaces};
use crate::{
    clock::ClockParams,
    config::Config,
    crypto::PasswordBox,
    depot::DepotWorkforce,
    events::EventSink,
    keycloak::{KcAdmin, ServiceToken, TokenManager},
    logs::LogHub,
    npc::{
        assign,
        manager::{NpcManager, Services},
        names::NameGenerator,
    },
    resources_dynamic::PlanetCatalogHandle,
    roles::RoleTemplates,
    schedule::{Schedule, ScheduleStore},
    schedule_import::{ImportMode, import_schedules},
    world_data::{WorldData, poi::VillageKind},
};

fn new_npc(n: usize) -> npcs::NewNpc {
    npcs::NewNpc {
        id: Uuid::new_v4(),
        login: format!("npc-{n:08x}"),
        password_enc: vec![0; 40],
        first_name: "Test".into(),
        last_name: format!("N{n}"),
        display_name: format!("Test N{n}"),
        enabled: true,
        role: "unassigned".into(),
        role_locked: false,
        village_code: None,
        schedule_pattern: None,
        action_profile: vec![],
        created_by: None,
    }
}

#[tokio::test]
async fn imports_the_shipped_schedules() {
    let Some(pool) = testdb::fresh().await else { return };
    let yaml = std::fs::read_to_string("config/schedules.yaml").unwrap();
    let file: crate::schedule::ScheduleFile = serde_yaml::from_str(&yaml).unwrap();

    let r = import_schedules(&pool, &yaml, ImportMode::Missing, Some("test")).await.unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(patterns::count(&pool).await.unwrap() as usize, file.schedules.len());
    assert!(r.created.iter().any(|c| c == "magasinier_1"));
    assert!(r.created.iter().any(|c| c == "activity_actions.work"));

    // The database copy behaves like the file.
    let store = ScheduleStore::new(Schedule::empty());
    store.reload(&pool).await.unwrap();
    let from_file = Schedule::from_yaml(&yaml).unwrap();
    assert_eq!(store.load().pattern_names(), from_file.pattern_names());
    assert_eq!(store.load().activity_actions(), from_file.activity_actions());

    // Missing mode keeps edits made from the panel.
    let edited = vec!["T".repeat(25); 5];
    patterns::upsert(&pool, "magasinier_1", None, &edited, "panel", Some("admin")).await.unwrap();
    let r = import_schedules(&pool, &yaml, ImportMode::Missing, None).await.unwrap();
    assert!(!r.changed());
    assert_eq!(patterns::get(&pool, "magasinier_1").await.unwrap().unwrap().days, edited);

    // Overwrite puts the file back.
    let r = import_schedules(&pool, &yaml, ImportMode::Overwrite, None).await.unwrap();
    assert!(r.updated.iter().any(|u| u == "magasinier_1"));
    assert_ne!(patterns::get(&pool, "magasinier_1").await.unwrap().unwrap().days, edited);

    // A bad pattern is reported, the others still land.
    let bad = "schedules:\n  ok_one:\n    days: [\"TTTT\"]\n  bad_one:\n    days: [\"TTxT\"]\n";
    let r = import_schedules(&pool, bad, ImportMode::Missing, None).await.unwrap();
    assert_eq!(r.created, ["ok_one"]);
    assert_eq!(r.errors.len(), 1);
}

#[tokio::test]
async fn pattern_in_use_cannot_be_deleted() {
    let Some(pool) = testdb::fresh().await else { return };
    patterns::upsert(&pool, "p1", None, &["TTTT".into()], "panel", None).await.unwrap();
    let mut n = new_npc(1);
    n.schedule_pattern = Some("p1".into());
    npcs::insert(&pool, &n).await.unwrap();
    assert!(matches!(patterns::delete(&pool, "p1").await, Err(crate::error::NpcError::Conflict(_))));
    npcs::delete(&pool, n.id).await.unwrap();
    patterns::delete(&pool, "p1").await.unwrap();
}

#[tokio::test]
async fn tasks_claim_in_order_and_cancel() {
    let Some(pool) = testdb::fresh().await else { return };
    let n = new_npc(1);
    npcs::insert(&pool, &n).await.unwrap();
    let t1 = tasks::create(&pool, n.id, "wait", &json!({"seconds": 5}), false, None).await.unwrap();
    let t2 = tasks::create(&pool, n.id, "wait", &json!({"seconds": 5}), true, None).await.unwrap();
    assert!(tasks::has_open_force_online(&pool, n.id).await.unwrap());
    assert_eq!(tasks::claim_next(&pool, n.id).await.unwrap().unwrap().id, t1.id);
    assert_eq!(tasks::cancel(&pool, t2.id).await.unwrap(), "pending");
    assert!(tasks::cancel(&pool, t2.id).await.is_err(), "already closed");
    assert!(tasks::claim_next(&pool, n.id).await.unwrap().is_none());
    tasks::finish(&pool, t1.id, "done", None).await.unwrap();
    assert_eq!(tasks::get(&pool, t1.id).await.unwrap().status, "done");
    assert_eq!(tasks::fail_orphans(&pool).await.unwrap(), 0);
}

#[tokio::test]
async fn events_batch_survives_a_deleted_npc_and_prunes() {
    let Some(pool) = testdb::fresh().await else { return };
    let n = new_npc(1);
    npcs::insert(&pool, &n).await.unwrap();
    let ev = |npc_id, days_ago: i64| events::NewEvent {
        npc_id,
        ts: time::OffsetDateTime::now_utc() - time::Duration::days(days_ago),
        kind: "test".into(),
        level: "info",
        message: "m".into(),
        data: None,
    };
    events::insert_batch(&pool, &[ev(Some(n.id), 0), ev(Some(Uuid::new_v4()), 0), ev(None, 10)]).await.unwrap();
    let all = events::list(&pool, None, &Default::default()).await.unwrap();
    assert_eq!(all.len(), 2, "the event of the unknown NPC is skipped");
    assert_eq!(events::prune(&pool, 7).await.unwrap(), 1);
}

async fn manager(pool: sqlx::PgPool) -> Arc<NpcManager> {
    let mut config = Config::from_env();
    config.keycloak_url = "http://127.0.0.1:1".into();
    let config = Arc::new(config);
    let http = reqwest::Client::new();
    let token = Arc::new(ServiceToken::new(http.clone(), config.token_url(), "svc-npc".into(), String::new()));
    let kc = Arc::new(KcAdmin::new(http.clone(), &config.keycloak_url, "dyingstar", Arc::clone(&token)));
    let passwords = PasswordBox::from_base64(&STANDARD.encode([3u8; 32])).unwrap();
    let schedule = Arc::new(ScheduleStore::new(Schedule::empty()));
    let yaml = std::fs::read_to_string("config/schedules.yaml").unwrap();
    import_schedules(&pool, &yaml, ImportMode::Missing, None).await.unwrap();
    schedule.reload(&pool).await.unwrap();
    let tokens = Arc::new(TokenManager::new(
        http,
        config.token_url(),
        "npc-login".into(),
        String::new(),
        pool.clone(),
        passwords.clone(),
        Arc::clone(&kc),
        EventSink::disabled(),
    ));
    let svc = Arc::new(Services {
        config,
        pool,
        clock_params: Arc::new(ClockParams::new(5, 0, None, 1.0)),
        planets: PlanetCatalogHandle::empty(),
        schedule,
        workforce: Arc::new(DepotWorkforce::new()),
        tokens,
        events: EventSink::disabled(),
    });
    NpcManager::new(
        svc,
        kc,
        None,
        Arc::new(RoleTemplates::load("config/roles.yaml").unwrap()),
        NameGenerator::new(vec![], vec![]),
        passwords,
        LogHub::new(10),
    )
}

#[tokio::test]
async fn first_five_of_a_village_become_its_storekeepers() {
    let Some(pool) = testdb::fresh().await else { return };
    let wd = WorldData::shared().expect("world data in config/world");
    let village = wd.villages.of_kind(VillageKind::Mining).next().expect("a Mining village");
    let code = village.name.clone();
    let pos = wd.village_position(village);
    let depot = Uuid::new_v4();
    workplaces::upsert(&pool, &code, depot, Some("WH-1"), None, "manual", None).await.unwrap();

    let mgr = manager(pool.clone()).await;
    let mut ids = Vec::new();
    for i in 0..8 {
        ids.push(npcs::insert(&pool, &new_npc(i)).await.unwrap().id);
    }
    // All eight report their position at once.
    let results = join_all(ids.iter().map(|id| assign::assign(&mgr, *id, pos, &[]))).await;
    assert!(results.iter().all(Result::is_ok), "{results:?}");

    let rows = npcs::all(&pool).await.unwrap();
    let mut slots: Vec<i16> = rows.iter().filter_map(|r| r.storekeeper_slot).collect();
    slots.sort();
    assert_eq!(slots, [1, 2, 3, 4, 5]);
    assert_eq!(rows.iter().filter(|r| r.role == "villager").count(), 3);
    for r in rows.iter().filter(|r| r.role == "storekeeper") {
        let spec = r.to_spec().unwrap();
        assert_eq!(spec.place_where_work.unwrap().uuid, depot.to_string());
        assert_eq!(spec.schedule.unwrap(), format!("magasinier_{}", r.storekeeper_slot.unwrap()));
        assert_eq!(r.village_code.as_deref(), Some(code.as_str()));
    }
    // Already assigned: a second report changes nothing.
    assign::assign(&mgr, ids[0], pos, &[]).await.unwrap();
    assert_eq!(npcs::all(&pool).await.unwrap().iter().filter(|r| r.role == "storekeeper").count(), 5);

    // Deleting a storekeeper frees its slot for the next newcomer.
    let sk = rows.iter().find(|r| r.storekeeper_slot == Some(3)).unwrap();
    npcs::delete(&pool, sk.id).await.unwrap();
    let late = npcs::insert(&pool, &new_npc(99)).await.unwrap();
    assign::assign(&mgr, late.id, pos, &[]).await.unwrap();
    assert_eq!(npcs::require(&pool, late.id).await.unwrap().storekeeper_slot, Some(3));
}

#[tokio::test]
async fn far_from_any_mining_village_is_a_villager() {
    let Some(pool) = testdb::fresh().await else { return };
    let mgr = manager(pool.clone()).await;
    let id = npcs::insert(&pool, &new_npc(1)).await.unwrap().id;
    // A point on the planet surface far from every Mining village.
    let wd = WorldData::shared().expect("world data in config/world");
    let r = wd.radius_m();
    let nowhere = (0..200)
        .map(|i| {
            let t = i as f64 * 2.399_963; // golden angle
            let z = 1.0 - 2.0 * (i as f64 + 0.5) / 200.0;
            let s = (1.0 - z * z).sqrt();
            crate::horizon::messages::Vec3 { x: r * s * t.cos(), y: r * z, z: r * s * t.sin() }
        })
        .find(|p| wd.home_mining_village(*p).is_none())
        .expect("somewhere outside every village");
    assign::assign(&mgr, id, nowhere, &[]).await.unwrap();
    let row = npcs::require(&pool, id).await.unwrap();
    assert_eq!(row.role, "villager");
    assert!(row.village_code.is_none());
}

#[tokio::test]
async fn no_warehouse_keeps_the_assignment_pending() {
    let Some(pool) = testdb::fresh().await else { return };
    let wd = WorldData::shared().expect("world data in config/world");
    let village = wd.villages.of_kind(VillageKind::Mining).next().unwrap();
    let mgr = manager(pool.clone()).await;
    let id = npcs::insert(&pool, &new_npc(1)).await.unwrap().id;
    assign::assign(&mgr, id, wd.village_position(village), &[]).await.unwrap();
    let row = npcs::require(&pool, id).await.unwrap();
    assert!(row.assignment_pending(), "retried at the next session");
    assert_eq!(row.village_code.as_deref(), Some(village.name.as_str()));
    assert!(row.to_spec().unwrap().take_a_walk.is_some(), "strolls meanwhile");
}
