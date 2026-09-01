use std::collections::{BTreeMap, HashMap};

use nimino_boundary::{
    WorkflowAction, WorkflowActionKind, WorkflowDefinition, WorkflowPlanRequest, WorkflowRunState,
    WorkflowRunStatus, WorkflowStep, WorkflowTrigger, WorkflowTriggerKind,
};
use serde_json::Value;

use crate::{executor::TriggerContext, ActionDef, TriggerDef, WorkflowDef};

pub(crate) enum PlannedStep {
    Skip,
    Complete,
    Execute(ActionDef),
}

pub(crate) fn definition(definition: &WorkflowDef) -> WorkflowDefinition {
    WorkflowDefinition {
        name: definition.name.clone(),
        description: definition.description.clone().unwrap_or_default(),
        trigger: trigger(&definition.trigger),
        steps: definition
            .steps
            .iter()
            .map(|step| WorkflowStep {
                id: step.id.clone(),
                name: step.name.clone().unwrap_or_default(),
                condition: step.if_expr.clone().unwrap_or_default(),
                timeout_secs: step.timeout_secs.unwrap_or_default(),
                action: action(&step.action),
            })
            .collect(),
        enabled: definition.enabled,
    }
}

fn trigger(trigger: &TriggerDef) -> WorkflowTrigger {
    let (kind, filter, emoji, cron, interval) = match trigger {
        TriggerDef::MessagePosted { filter } => (
            WorkflowTriggerKind::MessagePosted,
            filter.clone().unwrap_or_default(),
            String::new(),
            String::new(),
            String::new(),
        ),
        TriggerDef::ReactionAdded { emoji, filter } => (
            WorkflowTriggerKind::ReactionAdded,
            filter.clone().unwrap_or_default(),
            emoji.clone().unwrap_or_default(),
            String::new(),
            String::new(),
        ),
        TriggerDef::DiffPosted { filter } => (
            WorkflowTriggerKind::DiffPosted,
            filter.clone().unwrap_or_default(),
            String::new(),
            String::new(),
            String::new(),
        ),
        TriggerDef::Schedule { cron, interval } => (
            WorkflowTriggerKind::Schedule,
            String::new(),
            String::new(),
            cron.clone().unwrap_or_default(),
            interval.clone().unwrap_or_default(),
        ),
        TriggerDef::Webhook => (
            WorkflowTriggerKind::Webhook,
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ),
    };
    WorkflowTrigger {
        kind,
        filter,
        emoji,
        cron,
        interval,
    }
}

fn action(action: &ActionDef) -> WorkflowAction {
    let mut result = WorkflowAction {
        kind: WorkflowActionKind::Delay,
        text: String::new(),
        channel: String::new(),
        reply_in_thread: false,
        recipient: String::new(),
        topic: String::new(),
        emoji: String::new(),
        url: String::new(),
        http_method: String::new(),
        headers: BTreeMap::new(),
        body: String::new(),
        approver: String::new(),
        message: String::new(),
        timeout: String::new(),
        duration: String::new(),
    };
    match action {
        ActionDef::SendMessage {
            text,
            channel,
            reply_in_thread,
        } => {
            result.kind = WorkflowActionKind::SendMessage;
            result.text.clone_from(text);
            result.channel = channel.clone().unwrap_or_default();
            result.reply_in_thread = *reply_in_thread;
        }
        ActionDef::SendDm { to, text } => {
            result.kind = WorkflowActionKind::SendDm;
            result.recipient.clone_from(to);
            result.text.clone_from(text);
        }
        ActionDef::SetChannelTopic { topic } => {
            result.kind = WorkflowActionKind::SetChannelTopic;
            result.topic.clone_from(topic);
        }
        ActionDef::AddReaction { emoji } => {
            result.kind = WorkflowActionKind::AddReaction;
            result.emoji.clone_from(emoji);
        }
        ActionDef::CallWebhook {
            url,
            method,
            headers,
            body,
        } => {
            result.kind = WorkflowActionKind::CallWebhook;
            result.url.clone_from(url);
            result.http_method = method.clone().unwrap_or_default();
            result.headers = headers.clone().unwrap_or_default().into_iter().collect();
            result.body = body.clone().unwrap_or_default();
        }
        ActionDef::RequestApproval {
            from,
            message,
            timeout,
        } => {
            result.kind = WorkflowActionKind::RequestApproval;
            result.approver.clone_from(from);
            result.message.clone_from(message);
            result.timeout = timeout.clone().unwrap_or_default();
        }
        ActionDef::Delay { duration } => {
            result.duration.clone_from(duration);
        }
    }
    result
}

pub(crate) fn adapter_action(action: WorkflowAction) -> ActionDef {
    match action.kind {
        WorkflowActionKind::SendMessage => ActionDef::SendMessage {
            text: action.text,
            channel: (!action.channel.is_empty()).then_some(action.channel),
            reply_in_thread: action.reply_in_thread,
        },
        WorkflowActionKind::SendDm => ActionDef::SendDm {
            to: action.recipient,
            text: action.text,
        },
        WorkflowActionKind::SetChannelTopic => ActionDef::SetChannelTopic {
            topic: action.topic,
        },
        WorkflowActionKind::AddReaction => ActionDef::AddReaction {
            emoji: action.emoji,
        },
        WorkflowActionKind::CallWebhook => ActionDef::CallWebhook {
            url: action.url,
            method: (!action.http_method.is_empty()).then_some(action.http_method),
            headers: (!action.headers.is_empty()).then(|| action.headers.into_iter().collect()),
            body: (!action.body.is_empty()).then_some(action.body),
        },
        WorkflowActionKind::RequestApproval => ActionDef::RequestApproval {
            from: action.approver,
            message: action.message,
            timeout: (!action.timeout.is_empty()).then_some(action.timeout),
        },
        WorkflowActionKind::Delay => ActionDef::Delay {
            duration: action.duration,
        },
    }
}

pub(crate) fn plan_request(
    definition_value: &WorkflowDef,
    trigger: &TriggerContext,
    step_outputs: &HashMap<String, Value>,
    step_index: usize,
) -> WorkflowPlanRequest {
    let mut trigger_values = trigger
        .webhook_fields
        .iter()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect::<HashMap<_, _>>();
    trigger_values.extend([
        ("text".to_owned(), Value::String(trigger.text.clone())),
        ("author".to_owned(), Value::String(trigger.author.clone())),
        (
            "channel_id".to_owned(),
            Value::String(trigger.channel_id.clone()),
        ),
        (
            "timestamp".to_owned(),
            Value::String(trigger.timestamp.clone()),
        ),
        ("emoji".to_owned(), Value::String(trigger.emoji.clone())),
        (
            "message_id".to_owned(),
            Value::String(trigger.message_id.clone()),
        ),
        ("is_reply".to_owned(), Value::Bool(trigger.is_reply)),
    ]);
    WorkflowPlanRequest {
        definition: definition(definition_value),
        state: WorkflowRunState {
            status: WorkflowRunStatus::Running,
            current_step: u32::try_from(step_index).unwrap_or(u32::MAX),
            revision: 0,
        },
        bound_channel: trigger.channel_id.clone(),
        trigger: trigger_values,
        step_outputs: step_outputs
            .iter()
            .filter_map(|(step, output)| {
                output
                    .as_object()
                    .map(|fields| (step.clone(), fields.clone().into_iter().collect()))
            })
            .collect(),
    }
}

pub(crate) fn condition_values(
    trigger: &TriggerContext,
    step_outputs: &HashMap<String, Value>,
) -> HashMap<String, Value> {
    let mut values = HashMap::from([
        (
            "trigger_text".to_owned(),
            Value::String(trigger.text.clone()),
        ),
        (
            "trigger_author".to_owned(),
            Value::String(trigger.author.clone()),
        ),
        (
            "trigger_channel_id".to_owned(),
            Value::String(trigger.channel_id.clone()),
        ),
        (
            "trigger_timestamp".to_owned(),
            Value::String(trigger.timestamp.clone()),
        ),
        (
            "trigger_emoji".to_owned(),
            Value::String(trigger.emoji.clone()),
        ),
        (
            "trigger_message_id".to_owned(),
            Value::String(trigger.message_id.clone()),
        ),
        ("trigger_is_reply".to_owned(), Value::Bool(trigger.is_reply)),
    ]);
    for (key, value) in &trigger.webhook_fields {
        if !key.starts_with("trigger_")
            && !key.starts_with("steps_")
            && ![
                "text",
                "author",
                "channel_id",
                "timestamp",
                "emoji",
                "message_id",
                "is_reply",
            ]
            .contains(&key.as_str())
        {
            values.insert(format!("trigger_{key}"), Value::String(value.clone()));
        }
    }
    for (step_id, output) in step_outputs {
        if let Value::Object(fields) = output {
            for (field, value) in fields {
                values.insert(format!("steps_{step_id}_output_{field}"), value.clone());
            }
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_definition_maps_to_typed_policy_facts() {
        let definition_value = WorkflowDef {
            name: "notify".to_owned(),
            description: None,
            trigger: TriggerDef::MessagePosted {
                filter: Some("trigger_is_reply == false".to_owned()),
            },
            steps: vec![crate::Step {
                id: "post".to_owned(),
                name: None,
                if_expr: None,
                timeout_secs: Some(5),
                action: ActionDef::SendMessage {
                    text: "hello".to_owned(),
                    channel: None,
                    reply_in_thread: true,
                },
            }],
            enabled: true,
        };

        let mapped = definition(&definition_value);
        assert_eq!(mapped.trigger.kind, WorkflowTriggerKind::MessagePosted);
        assert_eq!(mapped.trigger.filter, "trigger_is_reply == false");
        assert_eq!(mapped.steps[0].action.kind, WorkflowActionKind::SendMessage);
        assert!(mapped.steps[0].action.reply_in_thread);
        assert_eq!(mapped.steps[0].timeout_secs, 5);
    }
}
