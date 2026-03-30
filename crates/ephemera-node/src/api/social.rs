//! JSON-RPC handlers for social, messages, profiles, and moderation namespaces.
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

/// Register `social.*` namespace methods.
pub fn register_social(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("social.connect", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            let message = params.get("message").and_then(|v| v.as_str()).unwrap_or("");
            // Get the network reference for gossip publication (clone Arc outside lock).
            let net_arc = {
                let guard = svc.network.lock().map_err(|e| internal_error(format!("lock: {e}")))?;
                guard.clone()
            };
            let net_ref = net_arc.as_deref();
            svc.social
                .connect(&target, message, &svc.identity, net_ref, Some(&svc.dht_storage), Some(&svc.metadata_db))
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.accept", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let from = extract_str(&params, "from")?;
            let net_arc = {
                let guard = svc.network.lock().map_err(|e| internal_error(format!("lock: {e}")))?;
                guard.clone()
            };
            let net_ref = net_arc.as_deref();
            let result = svc.social
                .accept(&from, &svc.identity, net_ref, Some(&svc.dht_storage), Some(&svc.metadata_db))
                .await
                .map_err(internal_error)?;

            // Emit a ConnectionAccepted notification (best-effort).
            let display_name = result.get("display_name").and_then(|v| v.as_str());
            let from_bytes = hex::decode(&from).ok();
            let _ = crate::services::NotificationService::insert(
                &svc.metadata_db,
                "connection_accepted",
                from_bytes.as_deref(),
                display_name,
                Some("You accepted a connection request"),
                None,
            );
            svc.event_bus.emit(Event::ConnectionAccepted {
                peer: from.clone(),
            });
            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.reject", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let from = extract_str(&params, "from")?;
            svc.social
                .reject(&from, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.disconnect", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            svc.social
                .disconnect(&target, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.cancel_request", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            svc.social
                .cancel_request(&target, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.resend_request", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            let message = params.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let net_arc = {
                let guard = svc.network.lock().map_err(|e| internal_error(format!("lock: {e}")))?;
                guard.clone()
            };
            let net_ref = net_arc.as_deref();
            svc.social
                .resend_request(&target, message, &svc.identity, net_ref, Some(&svc.dht_storage), Some(&svc.metadata_db))
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.list_connections", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let status = params
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("all");
            let mut result = svc.social
                .list_connections(status, &svc.identity, Some(&svc.metadata_db))
                .await
                .map_err(internal_error)?;
            // Enrich connections with handle from registry.
            if let Some(conns) = result.get_mut("connections").and_then(|v| v.as_array_mut()) {
                if let Ok(reg) = svc.handle_registry.lock() {
                    for conn in conns.iter_mut() {
                        if let Some(pid) = conn.get("pseudonym_id").and_then(|v| v.as_str()) {
                            if let Ok(bytes) = hex::decode(pid) {
                                if bytes.len() == 32 {
                                    let mut arr = [0u8; 32];
                                    arr.copy_from_slice(&bytes);
                                    let ik = ephemera_types::IdentityKey::from_bytes(arr);
                                    if let Some(h) = reg.lookup_by_owner(&ik) {
                                        if !h.is_expired() {
                                            if let Some(obj) = conn.as_object_mut() {
                                                obj.insert(
                                                    "handle".to_string(),
                                                    Value::String(h.name.clone()),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.follow", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            svc.social
                .follow(&target, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.unfollow", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            svc.social
                .unfollow(&target, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.react", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let hash = extract_str(&params, "content_hash")?;
            let emoji = extract_str(&params, "emoji")?;
            let action = params
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("add");
            svc.social
                .react(&hash, &emoji, action, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("social.get_reactions", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let hash = extract_str(&params, "content_hash")?;
            svc.social
                .get_reactions(&hash, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });
}

/// Register `messages.*` namespace methods.
pub fn register_messages(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("messages.send", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let recipient = extract_str(&params, "recipient")?;
            let body = extract_str(&params, "body")?;
            let ttl = params.get("ttl_seconds").and_then(|v| v.as_u64());

            // Get the network reference if available (set after node.start()).
            // Clone the Arc outside the lock to avoid holding MutexGuard across await.
            let net_arc = {
                let guard = svc.network.lock().map_err(|e| internal_error(format!("lock: {e}")))?;
                guard.clone()
            };
            let net_ref = net_arc.as_deref();

            let result = svc.messages
                .send(&recipient, &body, ttl, &svc.identity, net_ref, Some(&svc.dht_storage))
                .await
                .map_err(internal_error)?;

            // Emit MessageSent event (gap 3.3).
            if let Some(msg_hash) = result.get("message_hash").and_then(|v| v.as_str()) {
                if let Ok(recip_bytes) = hex::decode(&recipient) {
                    if recip_bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&recip_bytes);
                        svc.event_bus.emit(Event::MessageSent {
                            to: ephemera_types::IdentityKey::from_bytes(arr),
                            message_id: msg_hash.to_string(),
                        });
                    }
                }
            }

            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("messages.list_conversations", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            let mut result = svc.messages
                .list_conversations(&svc.identity, Some(&svc.metadata_db))
                .await
                .map_err(internal_error)?;
            // Enrich conversations with peer_handle from handle registry.
            if let Some(convs) = result.get_mut("conversations").and_then(|v| v.as_array_mut()) {
                if let Ok(reg) = svc.handle_registry.lock() {
                    for conv in convs.iter_mut() {
                        if let Some(peer_hex) = conv.get("peer").and_then(|v| v.as_str()) {
                            if let Ok(peer_bytes) = hex::decode(peer_hex) {
                                if peer_bytes.len() == 32 {
                                    let mut arr = [0u8; 32];
                                    arr.copy_from_slice(&peer_bytes);
                                    let identity_key = ephemera_types::IdentityKey::from_bytes(arr);
                                    if let Some(handle) = reg.lookup_by_owner(&identity_key) {
                                        if !handle.is_expired() {
                                            if let Some(obj) = conv.as_object_mut() {
                                                obj.insert(
                                                    "peer_handle".to_string(),
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
            }
            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("messages.get_thread", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let conv_id = extract_str(&params, "conversation_id")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            svc.messages
                .get_thread(&conv_id, limit, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("messages.mark_read", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let conv_id = extract_str(&params, "conversation_id")?;
            svc.messages
                .mark_read(&conv_id)
                .await
                .map_err(internal_error)
        }
    });
}

/// Register `profiles.*` namespace methods.
pub fn register_profiles(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("profiles.get", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let pubkey = extract_str(&params, "pubkey")?;
            svc.profiles
                .get_with_dht(&pubkey, &svc.metadata_db, &svc.dht_storage)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("profiles.update", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = params.get("display_name").and_then(|v| v.as_str());
            let bio = params.get("bio").and_then(|v| v.as_str());
            svc.profiles
                .update_and_publish(
                    name, bio, &svc.identity, &svc.metadata_db, &svc.dht_storage,
                )
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("profiles.get_mine", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            svc.profiles
                .get_mine(&svc.identity, &svc.metadata_db, &svc.handle_registry)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("profiles.update_avatar", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let data_hex = extract_str(&params, "data_hex")?;
            let _filename = extract_str(&params, "filename")?;
            let data = hex::decode(&data_hex).map_err(|e| JsonRpcError {
                code: error_codes::INVALID_PARAMS,
                message: format!("invalid hex data: {e}"),
                data: None,
            })?;
            svc.profiles
                .update_avatar(
                    &data,
                    &_filename,
                    &svc.identity,
                    &svc.metadata_db,
                    svc.content_store(),
                )
                .await
                .map_err(internal_error)
        }
    });
}

/// Register `moderation.*` namespace methods.
pub fn register_moderation(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("moderation.report", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let hash = extract_str(&params, "content_hash")?;
            let reason = extract_str(&params, "reason")?;
            svc.moderation
                .report(
                    &hash,
                    &reason,
                    &svc.identity,
                    &svc.posts,
                    &svc.metadata_db,
                    &svc.reputation,
                )
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("moderation.block", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            svc.moderation
                .block(&target, &svc.social, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("moderation.unblock", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            svc.moderation
                .unblock(&target, &svc.social, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("moderation.mute", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            let duration = params.get("duration_hours").and_then(|v| v.as_u64());
            svc.moderation
                .mute(&target, duration, &svc.social, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("moderation.unmute", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let target = extract_str(&params, "target")?;
            svc.moderation
                .unmute(&target, &svc.social, &svc.identity)
                .await
                .map_err(internal_error)
        }
    });
}

/// Register `groups.*` namespace methods.
pub fn register_groups(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("groups.create", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = extract_str(&params, "name")?;
            let desc = params.get("description").and_then(|v| v.as_str());
            let vis = params.get("visibility").and_then(|v| v.as_str()).unwrap_or("public");
            svc.social.create_group(&name, desc, vis, &svc.identity, &svc.rate_limiter).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.register_handle", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            let handle = extract_str(&params, "handle")?;
            svc.social.register_group_handle(&gid, &handle, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.join", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            svc.social.join_group(&gid, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.invite", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            let target = extract_str(&params, "target")?;
            svc.social.invite_to_group(&gid, &target, &svc.identity, &svc.rate_limiter).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.leave", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            svc.social.leave_group(&gid, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.set_role", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            let target = extract_str(&params, "target")?;
            let role = extract_str(&params, "role")?;
            svc.social.set_group_role(&gid, &target, &role, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.kick", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            let target = extract_str(&params, "target")?;
            svc.social.kick_member(&gid, &target, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.ban", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            let target = extract_str(&params, "target")?;
            let reason = params.get("reason").and_then(|v| v.as_str());
            svc.social.ban_member(&gid, &target, reason, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.post", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            let ch = extract_str(&params, "content_hash")?;
            svc.social.post_to_group(&gid, &ch, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.feed", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
            svc.social.get_group_feed(&gid, limit, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.list", move |_params| {
        let svc = Arc::clone(&svc);
        async move { svc.social.list_my_groups(&svc.identity).await.map_err(internal_error) }
    });

    let svc = Arc::clone(services);
    router.register("groups.search", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let query = extract_str(&params, "query")?;
            svc.social.search_groups(&query).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.info", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            svc.social.get_group_info(&gid, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.delete", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            svc.social.delete_group(&gid, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.transfer_ownership", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            let target = extract_str(&params, "target")?;
            svc.social.transfer_ownership(&gid, &target, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("groups.delete_post", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            let ch = extract_str(&params, "content_hash")?;
            svc.social.delete_group_post(&gid, &ch, &svc.identity).await.map_err(internal_error)
        }
    });
}

/// Register `group_chats.*` namespace methods.
pub fn register_group_chats(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("group_chats.create_private", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = params.get("name").and_then(|v| v.as_str());
            let members: Vec<String> = params.get("members")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            svc.social.create_private_chat(name, &members, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("group_chats.create_linked", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let gid = extract_str(&params, "group_id")?;
            svc.social.create_group_chat(&gid).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("group_chats.add_member", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let cid = extract_str(&params, "chat_id")?;
            let member = extract_str(&params, "member")?;
            svc.social.add_chat_member(&cid, &member, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("group_chats.leave", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let cid = extract_str(&params, "chat_id")?;
            svc.social.leave_chat(&cid, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("group_chats.send", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let cid = extract_str(&params, "chat_id")?;
            let body = extract_str(&params, "body")?;
            svc.social.send_group_chat_message(&cid, &body, &svc.identity, &svc.rate_limiter).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("group_chats.messages", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let cid = extract_str(&params, "chat_id")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
            svc.social.get_chat_messages(&cid, limit, &svc.identity).await.map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("group_chats.list", move |_params| {
        let svc = Arc::clone(&svc);
        async move { svc.social.list_my_group_chats(&svc.identity).await.map_err(internal_error) }
    });
}

/// Register `mentions.*` namespace methods.
pub fn register_mentions(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("mentions.list", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
            svc.social.list_mentions(&svc.identity, limit).await.map_err(internal_error)
        }
    });
}

/// Register `topics.*` namespace methods.
pub fn register_topics(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("topics.create", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = extract_str(&params, "name")?;
            let desc = params.get("description").and_then(|v| v.as_str());
            svc.social.create_topic(&name, desc, &svc.identity).await.map_err(internal_error)
        }
    });
    let svc = Arc::clone(services);
    router.register("topics.join", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let tid = extract_str(&params, "topic_id")?;
            svc.social.join_topic(&tid, &svc.identity).await.map_err(internal_error)
        }
    });
    let svc = Arc::clone(services);
    router.register("topics.leave", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let tid = extract_str(&params, "topic_id")?;
            svc.social.leave_topic(&tid, &svc.identity).await.map_err(internal_error)
        }
    });
    let svc = Arc::clone(services);
    router.register("topics.list", move |_params| {
        let svc = Arc::clone(&svc);
        async move { svc.social.list_topics().await.map_err(internal_error) }
    });
    let svc = Arc::clone(services);
    router.register("topics.feed", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let tid = extract_str(&params, "topic_id")?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
            svc.social.get_topic_feed(&tid, limit).await.map_err(internal_error)
        }
    });
    let svc = Arc::clone(services);
    router.register("topics.post", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let tid = extract_str(&params, "topic_id")?;
            let ch = extract_str(&params, "content_hash")?;
            svc.social.post_to_topic(&tid, &ch).await.map_err(internal_error)
        }
    });
}

/// Register `notifications.*` namespace methods.
pub fn register_notifications(router: &mut Router, services: &Arc<ServiceContainer>) {
    use crate::services::NotificationService;

    let svc = Arc::clone(services);
    router.register("notifications.list", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
            NotificationService::list_unread(&svc.metadata_db, limit).map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("notifications.count", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            NotificationService::count_unread(&svc.metadata_db).map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("notifications.mark_read", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let id = extract_str(&params, "notification_id")?;
            NotificationService::mark_read(&svc.metadata_db, &id).map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("notifications.mark_all_read", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            NotificationService::mark_all_read(&svc.metadata_db).map_err(internal_error)
        }
    });
}
