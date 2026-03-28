//! JSON-RPC handlers for posts, feed, and media namespaces.
//!
//! Split from [`crate::api`] to keep file sizes manageable.

use crate::rpc::{error_codes, JsonRpcError, Router};
use crate::services::ServiceContainer;
use ephemera_events::Event;
use serde_json::Value;
use std::sync::Arc;

/// Extract a required string parameter from the JSON-RPC params.
fn extract_str(params: &Value, key: &str) -> Result<String, JsonRpcError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| JsonRpcError {
            code: error_codes::INVALID_PARAMS,
            message: format!("missing or invalid parameter: {key}"),
            data: None,
        })
}

/// Convert a service error to a JSON-RPC internal error.
fn internal_error(msg: String) -> JsonRpcError {
    JsonRpcError {
        code: error_codes::INTERNAL_ERROR,
        message: msg,
        data: None,
    }
}

/// Extract media files from JSON-RPC params.
fn extract_media_files(params: &Value) -> Vec<crate::services::MediaFile> {
    let Some(media_arr) = params.get("media").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    media_arr
        .iter()
        .filter_map(|item| {
            let data_hex = item.get("data_hex")?.as_str()?;
            let filename = item.get("filename")?.as_str()?;
            let data = hex::decode(data_hex).ok()?;
            Some(crate::services::MediaFile {
                data,
                filename: filename.to_string(),
            })
        })
        .collect()
}

/// Enrich feed result posts with `author_handle` from the handle registry.
///
/// Iterates over `result["posts"]` and for each post that has an `author` hex
/// field, looks up the handle in the registry and inserts `author_handle`.
fn enrich_posts_with_handles(
    result: &mut Value,
    handle_registry: &std::sync::Mutex<ephemera_social::handle::HandleRegistry>,
) {
    let posts = match result.get_mut("posts").and_then(|v| v.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };
    let reg = match handle_registry.lock() {
        Ok(r) => r,
        Err(_) => return,
    };
    for post in posts.iter_mut() {
        if let Some(author_hex) = post.get("author").and_then(|v| v.as_str()) {
            if let Ok(author_bytes) = hex::decode(author_hex) {
                if author_bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&author_bytes);
                    let identity_key = ephemera_types::IdentityKey::from_bytes(arr);
                    if let Some(handle) = reg.lookup_by_owner(&identity_key) {
                        if !handle.is_expired() {
                            if let Some(obj) = post.as_object_mut() {
                                obj.insert(
                                    "author_handle".to_string(),
                                    Value::String(handle.name.clone()),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Register `posts.*` namespace methods.
pub fn register_posts(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("posts.create", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let body = extract_str(&params, "body")?;
            let ttl = params.get("ttl_seconds").and_then(|v| v.as_u64());
            let parent = params
                .get("parent")
                .and_then(|v| v.as_str())
                .map(String::from);
            let audience = params
                .get("audience")
                .and_then(|v| v.as_str())
                .unwrap_or("public")
                .to_string();
            let media_files = extract_media_files(&params);
            let result = svc.posts
                .create(
                    &body,
                    media_files,
                    ttl,
                    parent.as_deref(),
                    &svc.identity,
                    svc.content_store(),
                    &svc.metadata_db,
                    &svc.rate_limiter,
                    &svc.reputation,
                    &svc.fingerprint_store,
                    &svc.content_filter,
                    Some(&svc.epoch_key_manager),
                )
                .await
                .map_err(internal_error)?;

            // Emit PostCreated event (gap 3.3).
            if let Some(hash_hex) = result.get("content_hash").and_then(|v| v.as_str()) {
                if let Ok(hash_bytes) = hex::decode(hash_hex) {
                    if hash_bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&hash_bytes);
                        svc.event_bus.emit(Event::PostCreated {
                            content_id: ephemera_types::ContentId::from_digest(arr),
                        });
                    }
                }
            }

            // If audience is a topic, also post to that topic room.
            if let Some(topic_name) = audience.strip_prefix("topic:") {
                if let Some(hash_hex) = result.get("content_hash").and_then(|v| v.as_str()) {
                    let _ = svc.social.post_to_topic(topic_name, hash_hex).await;
                }
            }

            // Include the audience in the response.
            let mut result = result;
            if let Some(obj) = result.as_object_mut() {
                obj.insert("audience".to_string(), serde_json::Value::String(audience));
            }

            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("posts.get", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let hash = extract_str(&params, "content_hash")?;
            svc.posts
                .get(&hash, svc.content_store(), &svc.metadata_db)
                .await
                .map_err(|_| JsonRpcError {
                    code: error_codes::CONTENT_NOT_FOUND,
                    message: "post not found".into(),
                    data: None,
                })
        }
    });

    let svc = Arc::clone(services);
    router.register("posts.delete", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let hash = extract_str(&params, "content_hash")?;
            // Clone the Arc outside the lock to avoid holding MutexGuard across await.
            let network_arc = {
                let guard = svc.network.lock().map_err(|e| internal_error(format!("lock: {e}")))?;
                guard.clone()
            };
            let network_ref = network_arc.as_deref();
            svc.posts
                .delete_and_propagate(&hash, &svc.identity, &svc.metadata_db, network_ref)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("posts.list_by_author", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let author = extract_str(&params, "author")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            svc.posts
                .list_by_author(&author, limit, &svc.metadata_db)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("posts.replies", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let parent = extract_str(&params, "parent_hash")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            svc.posts
                .replies(&parent, limit, &svc.metadata_db)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("posts.reply", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let body = extract_str(&params, "body")?;
            let parent = extract_str(&params, "parent")?;
            let ttl = params.get("ttl_seconds").and_then(|v| v.as_u64());
            let media_files = extract_media_files(&params);
            let result = svc.posts
                .create(
                    &body,
                    media_files,
                    ttl,
                    Some(&parent),
                    &svc.identity,
                    svc.content_store(),
                    &svc.metadata_db,
                    &svc.rate_limiter,
                    &svc.reputation,
                    &svc.fingerprint_store,
                    &svc.content_filter,
                    Some(&svc.epoch_key_manager),
                )
                .await
                .map_err(internal_error)?;

            // Emit PostCreated event.
            if let Some(hash_hex) = result.get("content_hash").and_then(|v| v.as_str()) {
                if let Ok(hash_bytes) = hex::decode(hash_hex) {
                    if hash_bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&hash_bytes);
                        svc.event_bus.emit(Event::PostCreated {
                            content_id: ephemera_types::ContentId::from_digest(arr),
                        });
                    }
                }
            }

            Ok(result)
        }
    });
}

/// Register `feed.*` namespace methods.
pub fn register_feed(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("feed.connections", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            let cursor = params.get("after").and_then(|v| v.as_i64());
            let mut result = svc.feed
                .connections(
                    limit,
                    cursor,
                    &svc.metadata_db,
                    Some(&svc.identity),
                    Some(&svc.social.social_services),
                )
                .await
                .map_err(internal_error)?;
            enrich_posts_with_handles(&mut result, &svc.handle_registry);
            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("feed.discover", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            let mut result = svc.feed
                .discover(
                    limit,
                    &svc.metadata_db,
                    Some(&svc.identity),
                    Some(&svc.social.social_services),
                )
                .await
                .map_err(internal_error)?;
            enrich_posts_with_handles(&mut result, &svc.handle_registry);
            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("feed.topic", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let room_id = extract_str(&params, "room_id")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
            // Delegate to the real topic feed implementation instead of the stub.
            svc.social
                .get_topic_feed(&room_id, limit)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("feed.search_tags", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let tag = extract_str(&params, "tag")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            svc.feed
                .search_tags(&tag, limit, &svc.metadata_db)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("feed.profile", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let pubkey = extract_str(&params, "pubkey")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            svc.posts
                .list_by_author(&pubkey, limit, &svc.metadata_db)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("feed.own", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            // Get the active user's pubkey from the identity service.
            let active = svc.identity.get_active().await.map_err(internal_error)?;
            let pubkey = active
                .get("pubkey")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: error_codes::INTERNAL_ERROR,
                    message: "identity not available".into(),
                    data: None,
                })?
                .to_string();
            svc.posts
                .list_by_author(&pubkey, limit, &svc.metadata_db)
                .await
                .map_err(internal_error)
        }
    });
}

/// Register `media.*` namespace methods.
pub fn register_media(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("media.get_metadata", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let media_id = extract_str(&params, "media_id")?;
            crate::services::media::get_metadata(&media_id, &svc.metadata_db)
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("media.get_chunk", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let chunk_hash = extract_str(&params, "chunk_hash")?;
            crate::services::media::get_chunk(&chunk_hash, &svc.metadata_db)
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("media.get_thumbnail", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let media_id = extract_str(&params, "media_id")?;
            crate::services::media::get_thumbnail(
                &media_id,
                &svc.metadata_db,
                svc.content_store(),
            )
            .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("media.list_for_post", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let post_hash = extract_str(&params, "content_hash")?;
            crate::services::media::list_for_post(&post_hash, &svc.metadata_db)
                .map_err(internal_error)
        }
    });
}

/// Register `discover.*` namespace methods.
pub fn register_discover(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("discover.search", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let query = extract_str(&params, "query")?;
            let query_lower = query.to_lowercase();

            // Search handles in the local registry.
            let handles: Vec<serde_json::Value> = {
                let reg = svc.handle_registry.lock().map_err(|e| internal_error(format!("lock: {e}")))?;
                reg.search_prefix(&query_lower)
                    .into_iter()
                    .filter(|h| !h.is_expired())
                    .take(20)
                    .map(|h| {
                        serde_json::json!({
                            "handle": format!("@{}", h.name),
                            "owner": hex::encode(h.owner.as_bytes()),
                        })
                    })
                    .collect()
            };

            // Search topics.
            let topics: Vec<serde_json::Value> = {
                let all_topics = svc.social.list_topics().await.unwrap_or_else(|_| serde_json::json!({ "topics": [] }));
                all_topics["topics"]
                    .as_array()
                    .unwrap_or(&Vec::new())
                    .iter()
                    .filter(|t| {
                        t["name"]
                            .as_str()
                            .map(|n| n.to_lowercase().contains(&query_lower))
                            .unwrap_or(false)
                    })
                    .take(20)
                    .cloned()
                    .collect()
            };

            Ok(serde_json::json!({
                "handles": handles,
                "topics": topics,
            }))
        }
    });
}
