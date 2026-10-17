use crate::constants::PROFILE_STORAGE_KEY;
use crate::models::ctx::Ctx;
use crate::runtime::msg::{Action, ActionCtx};
use crate::runtime::{Env, EnvFutureExt, Runtime, RuntimeAction, RuntimeEvent, TryEnvFuture};
use crate::types::addon::{Descriptor, Manifest};
use crate::types::api::{APIRequest, APIResult, SuccessResponse};
use crate::types::events::DismissedEventsBucket;
use crate::types::library::LibraryBucket;
use crate::types::notifications::NotificationsBucket;
use crate::types::profile::{Auth, AuthKey, GDPRConsent, Profile, User};
use crate::types::search_history::SearchHistoryBucket;
use crate::types::server_urls::ServerUrlsBucket;
use crate::types::streams::StreamsBucket;
use crate::types::True;
use crate::unit_tests::{
    default_fetch_handler, Request, TestEnv, FETCH_HANDLER, REQUESTS, STORAGE,
};
use futures::channel::mpsc::Receiver;
use futures::future;
use semver::Version;
use std::any::Any;
use stremio_derive::Model;
use url::Url;

#[derive(Model, Clone, Default)]
#[model(TestEnv)]
struct TestModel {
    ctx: Ctx,
}

fn create_addon_descriptor(id: &str, transport_url: &str) -> Descriptor {
    Descriptor {
        manifest: Manifest {
            id: id.to_owned(),
            version: Version::new(0, 0, 1),
            name: "name".to_owned(),
            contact_email: None,
            description: None,
            logo: None,
            background: None,
            types: vec![],
            resources: vec![],
            id_prefixes: None,
            catalogs: vec![],
            addon_catalogs: vec![],
            behavior_hints: Default::default(),
        },
        transport_url: Url::parse(transport_url).unwrap(),
        flags: Default::default(),
    }
}

fn create_runtime(
    profile: Profile,
) -> (
    Runtime<TestEnv, TestModel>,
    Receiver<RuntimeEvent<TestEnv, TestModel>>,
) {
    STORAGE.write().unwrap().insert(
        PROFILE_STORAGE_KEY.to_owned(),
        serde_json::to_string(&profile).unwrap(),
    );
    Runtime::<TestEnv, _>::new(
        TestModel {
            ctx: Ctx::new(
                profile,
                LibraryBucket::default(),
                StreamsBucket::default(),
                ServerUrlsBucket::new::<TestEnv>(None),
                NotificationsBucket::new::<TestEnv>(None, vec![]),
                SearchHistoryBucket::default(),
                DismissedEventsBucket::default(),
            ),
        },
        vec![],
        1000,
    )
}

fn dispatch_move_addon(runtime: &Runtime<TestEnv, TestModel>, transport_url: &str, index: usize) {
    TestEnv::run(|| {
        runtime.dispatch(RuntimeAction {
            field: None,
            action: Action::Ctx(ActionCtx::MoveAddon {
                transport_url: Url::parse(transport_url).unwrap(),
                index,
            }),
        })
    });
}

fn stored_addons() -> Vec<Descriptor> {
    let storage = STORAGE.read().unwrap();
    serde_json::from_str::<Profile>(storage.get(PROFILE_STORAGE_KEY).unwrap())
        .unwrap()
        .addons
}

#[test]
fn actionctx_moveaddon_deserialize_js_shape() {
    let action = serde_json::from_value::<Action>(serde_json::json!({
        "action": "Ctx",
        "args": {
            "action": "MoveAddon",
            "args": {
                "transportUrl": "https://transport_url",
                "index": 1
            }
        }
    }))
    .expect("Should be a valid action");
    assert!(
        matches!(
            action,
            Action::Ctx(ActionCtx::MoveAddon { transport_url, index })
                if transport_url == Url::parse("https://transport_url").unwrap() && index == 1
        ),
        "MoveAddon action is deserialized from the camelCase JS shape"
    );
}

#[test]
fn actionctx_moveaddon_forward() {
    let addon_a = create_addon_descriptor("id_a", "https://transport_url_a");
    let addon_b = create_addon_descriptor("id_b", "https://transport_url_b");
    let addon_c = create_addon_descriptor("id_c", "https://transport_url_c");
    let _env_mutex = TestEnv::reset().expect("Should have exclusive lock to TestEnv");
    let (runtime, _rx) = create_runtime(Profile {
        addons: vec![addon_a.to_owned(), addon_b.to_owned(), addon_c.to_owned()],
        ..Default::default()
    });
    dispatch_move_addon(&runtime, "https://transport_url_a", 2);
    let expected = vec![addon_b, addon_c, addon_a];
    assert_eq!(
        runtime.model().unwrap().ctx.profile.addons,
        expected,
        "addon moved forward in memory"
    );
    assert_eq!(stored_addons(), expected, "addon moved forward in storage");
    assert!(
        REQUESTS.read().unwrap().is_empty(),
        "No requests have been sent"
    );
}

#[test]
fn actionctx_moveaddon_backward() {
    let addon_a = create_addon_descriptor("id_a", "https://transport_url_a");
    let addon_b = create_addon_descriptor("id_b", "https://transport_url_b");
    let addon_c = create_addon_descriptor("id_c", "https://transport_url_c");
    let _env_mutex = TestEnv::reset().expect("Should have exclusive lock to TestEnv");
    let (runtime, _rx) = create_runtime(Profile {
        addons: vec![addon_a.to_owned(), addon_b.to_owned(), addon_c.to_owned()],
        ..Default::default()
    });
    dispatch_move_addon(&runtime, "https://transport_url_c", 0);
    let expected = vec![addon_c, addon_a, addon_b];
    assert_eq!(
        runtime.model().unwrap().ctx.profile.addons,
        expected,
        "addon moved backward in memory"
    );
    assert_eq!(stored_addons(), expected, "addon moved backward in storage");
    assert!(
        REQUESTS.read().unwrap().is_empty(),
        "No requests have been sent"
    );
}

#[test]
fn actionctx_moveaddon_index_out_of_range_is_clamped() {
    let addon_a = create_addon_descriptor("id_a", "https://transport_url_a");
    let addon_b = create_addon_descriptor("id_b", "https://transport_url_b");
    let addon_c = create_addon_descriptor("id_c", "https://transport_url_c");
    let _env_mutex = TestEnv::reset().expect("Should have exclusive lock to TestEnv");
    let (runtime, _rx) = create_runtime(Profile {
        addons: vec![addon_a.to_owned(), addon_b.to_owned(), addon_c.to_owned()],
        ..Default::default()
    });
    dispatch_move_addon(&runtime, "https://transport_url_a", 10);
    let expected = vec![addon_b, addon_c, addon_a];
    assert_eq!(
        runtime.model().unwrap().ctx.profile.addons,
        expected,
        "addon moved to the last position in memory"
    );
    assert_eq!(
        stored_addons(),
        expected,
        "addon moved to the last position in storage"
    );
}

#[test]
fn actionctx_moveaddon_not_installed() {
    let addon_a = create_addon_descriptor("id_a", "https://transport_url_a");
    let addon_b = create_addon_descriptor("id_b", "https://transport_url_b");
    let _env_mutex = TestEnv::reset().expect("Should have exclusive lock to TestEnv");
    let (runtime, _rx) = create_runtime(Profile {
        addons: vec![addon_a.to_owned(), addon_b.to_owned()],
        ..Default::default()
    });
    dispatch_move_addon(&runtime, "https://transport_url_unknown", 0);
    let expected = vec![addon_a, addon_b];
    assert_eq!(
        runtime.model().unwrap().ctx.profile.addons,
        expected,
        "addons in memory not updated"
    );
    assert_eq!(stored_addons(), expected, "addons in storage not updated");
    assert!(
        REQUESTS.read().unwrap().is_empty(),
        "No requests have been sent"
    );
}

#[test]
fn actionctx_moveaddon_locked() {
    let addon_a = create_addon_descriptor("id_a", "https://transport_url_a");
    let addon_b = create_addon_descriptor("id_b", "https://transport_url_b");
    let _env_mutex = TestEnv::reset().expect("Should have exclusive lock to TestEnv");
    let (runtime, _rx) = create_runtime(Profile {
        addons: vec![addon_a.to_owned(), addon_b.to_owned()],
        addons_locked: true,
        ..Default::default()
    });
    dispatch_move_addon(&runtime, "https://transport_url_a", 1);
    let expected = vec![addon_a, addon_b];
    assert_eq!(
        runtime.model().unwrap().ctx.profile.addons,
        expected,
        "addons in memory not updated when locked"
    );
    assert_eq!(
        stored_addons(),
        expected,
        "addons in storage not updated when locked"
    );
    assert!(
        REQUESTS.read().unwrap().is_empty(),
        "No requests have been sent"
    );
}

#[test]
fn actionctx_moveaddon_with_user() {
    let addon_a = create_addon_descriptor("id_a", "https://transport_url_a");
    let addon_b = create_addon_descriptor("id_b", "https://transport_url_b");
    let expected = vec![addon_b.to_owned(), addon_a.to_owned()];
    let expected_body = serde_json::to_string(&APIRequest::AddonCollectionSet {
        auth_key: AuthKey("auth_key".to_owned()),
        addons: expected.to_owned(),
    })
    .unwrap();
    let fetch_handler_body = expected_body.to_owned();
    let _env_mutex = TestEnv::reset().expect("Should have exclusive lock to TestEnv");
    *FETCH_HANDLER.write().unwrap() = Box::new(
        move |request: Request| -> TryEnvFuture<Box<dyn Any + Send>> {
            match request {
                Request {
                    url, method, body, ..
                } if url == "https://api.strem.io/api/addonCollectionSet"
                    && method == "POST"
                    && body == fetch_handler_body =>
                {
                    future::ok(
                        Box::new(APIResult::Ok(SuccessResponse { success: True {} }))
                            as Box<dyn Any + Send>,
                    )
                    .boxed_env()
                }
                _ => default_fetch_handler(request),
            }
        },
    );
    let (runtime, _rx) = create_runtime(Profile {
        auth: Some(Auth {
            key: AuthKey("auth_key".to_owned()),
            user: User {
                id: "user_id".into(),
                email: "user_email".to_owned(),
                fb_id: None,
                apple_id: None,
                avatar: None,
                last_modified: TestEnv::now(),
                date_registered: TestEnv::now(),
                trakt: None,
                premium_expire: None,
                gdpr_consent: GDPRConsent {
                    tos: true,
                    privacy: true,
                    marketing: true,
                    from: Some("tests".to_owned()),
                },
                ..Default::default()
            },
        }),
        addons: vec![addon_a.to_owned(), addon_b.to_owned()],
        ..Default::default()
    });
    dispatch_move_addon(&runtime, "https://transport_url_a", 1);
    assert_eq!(
        runtime.model().unwrap().ctx.profile.addons,
        expected,
        "addon moved in memory"
    );
    assert_eq!(stored_addons(), expected, "addon moved in storage");
    assert_eq!(
        REQUESTS.read().unwrap().len(),
        1,
        "One request has been sent"
    );
    assert_eq!(
        REQUESTS.read().unwrap().first().unwrap().to_owned(),
        Request {
            url: "https://api.strem.io/api/addonCollectionSet".to_owned(),
            method: "POST".to_owned(),
            body: expected_body,
            ..Default::default()
        },
        "addonCollectionSet request with the new order has been sent"
    );
}
