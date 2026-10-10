//! The chief — one assistant face over the whole roster.
//!
//! The user talks to the chief, not to bots. `plan` asks the cheapest
//! available provider (via the forage router) what to do with a task:
//! answer directly, delegate to roster bots, or shrug — in which case
//! the deterministic routing table takes over. Delegation is data, not
//! a subsystem: a `Delegate` is just the next dispatch candidate.

use crate::forage::{ForageRequest, Router};
use crate::spec::AgentSpec;

/// One delegation the chief wants: a roster bot plus an optional
/// sub-task for it (`None` = the user's task verbatim).
#[derive(Debug, Clone, PartialEq)]
pub struct Delegate {
    pub bot: String,
    pub task: Option<String>,
}

/// What the chief decided to do with a task.
#[derive(Debug, Clone, PartialEq)]
pub enum ChiefPlan {
    /// Answer directly — no bot gets hatched.
    Answer(String),
    /// Hand work to roster bots, in order.
    Delegations(Vec<Delegate>),
    /// The planner couldn't decide — bad output, no provider — so the
    /// deterministic routing table takes the task instead.
    Undecided,
}

/// Ask the chief what to do with `task`. `context` is the recent
/// conversation (oldest→newest, `"role: text"` lines) so follow-ups
/// resolve; the roster is summarized as name + job — personas stay
/// server-side. Any planner trouble (no provider, malformed reply)
/// lands as `Undecided`, never an error.
pub fn plan(task: &str, context: &[String], agents: &[AgentSpec], router: &Router) -> ChiefPlan {
    let prompt = plan_prompt(task, context, agents);
    let Ok(res) = router.complete(&ForageRequest {
        prompt,
        model: None,
        tier: Some("concise".into()),
    }) else {
        return ChiefPlan::Undecided;
    };
    parse_plan(&res.text)
}

fn plan_prompt(task: &str, context: &[String], agents: &[AgentSpec]) -> String {
    let mut p = String::new();
    p.push_str(
        "You are the chief of a small team of bots. Decide what to do \
         with the user's request.\n\nRoster:\n",
    );
    for a in agents {
        p.push_str(&format!("- {}: {}\n", a.name, a.job));
    }
    if !context.is_empty() {
        p.push_str("\nRecent conversation:\n");
        for line in context {
            p.push_str(line);
            p.push('\n');
        }
    }
    p.push_str("\nReply with exactly one JSON object, no prose:\n");
    p.push_str(
        "- {\"answer\": \"<direct answer>\"} — for chat, questions, anything needing no bot\n\
         - {\"delegate\": [{\"bot\": \"<name>\", \"task\": \"<sub-task>\"}]} — hand work to roster bots\n\
         - {\"undecided\": true} — you cannot tell\n\n",
    );
    p.push_str(&format!("User request:\n{task}\n"));
    p
}

/// Parse the planner's reply. The object is first `{` to last `}` so
/// prose around the JSON doesn't matter; anything unreadable is
/// `Undecided`, not an error — a flaky provider must never break
/// dispatch.
pub fn parse_plan(text: &str) -> ChiefPlan {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return ChiefPlan::Undecided;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[start..=end]) else {
        return ChiefPlan::Undecided;
    };
    if let Some(answer) = v.get("answer").and_then(|a| a.as_str()) {
        return ChiefPlan::Answer(answer.trim().to_string());
    }
    if let Some(list) = v.get("delegate").and_then(|d| d.as_array()) {
        let delegates: Vec<Delegate> = list
            .iter()
            .filter_map(|d| {
                let bot = d.get("bot").and_then(|b| b.as_str())?.to_string();
                let task = d.get("task").and_then(|t| t.as_str()).map(str::to_string);
                Some(Delegate { bot, task })
            })
            .collect();
        if !delegates.is_empty() {
            return ChiefPlan::Delegations(delegates);
        }
    }
    ChiefPlan::Undecided
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn agent(name: &str, job: &str) -> AgentSpec {
        AgentSpec {
            name: name.into(),
            title: None,
            job: job.into(),
            model: None,
            mode: None,
            boss: false,
            max_steps: None,
            max_seconds: None,
            permissions: BTreeMap::new(),
            skills: vec![],
            mcp: vec![],
            budget: None,
            routines: Vec::new(),
            egress: None,
            body: String::new(),
        }
    }

    #[test]
    fn parses_an_answer() {
        assert_eq!(
            parse_plan(r#"{"answer": "the capital is Paris"}"#),
            ChiefPlan::Answer("the capital is Paris".into())
        );
    }

    #[test]
    fn parses_delegations() {
        assert_eq!(
            parse_plan(r#"{"delegate": [{"bot": "debugger", "task": "fix it"}, {"bot": "boss"}]}"#),
            ChiefPlan::Delegations(vec![
                Delegate {
                    bot: "debugger".into(),
                    task: Some("fix it".into()),
                },
                Delegate {
                    bot: "boss".into(),
                    task: None,
                },
            ])
        );
    }

    #[test]
    fn prose_wrapped_json_still_parses() {
        let reply = "Sure! Here's my plan:\n\n{\"answer\": \"hi\"}\nHope that helps";
        assert_eq!(parse_plan(reply), ChiefPlan::Answer("hi".into()));
    }

    #[test]
    fn garbage_is_undecided_not_an_error() {
        assert_eq!(parse_plan("I have no idea"), ChiefPlan::Undecided);
        assert_eq!(parse_plan("{}"), ChiefPlan::Undecided);
        assert_eq!(parse_plan(r#"{"delegate": []}"#), ChiefPlan::Undecided);
        assert_eq!(parse_plan(r#"{"undecided": true}"#), ChiefPlan::Undecided);
        // an unknown-bot delegation still parses — the brood filters it
        assert_eq!(
            parse_plan(r#"{"delegate": [{"bot": "ghost"}]}"#),
            ChiefPlan::Delegations(vec![Delegate {
                bot: "ghost".into(),
                task: None
            }])
        );
    }

    #[test]
    fn no_providers_leaves_the_task_to_the_routes() {
        // EchoForage echoes the prompt back — unparseable → Undecided.
        let router = Router::new(vec![]);
        let agents = vec![agent("doc-keeper", "keeps docs tidy")];
        assert_eq!(
            plan("sync the docs", &[], &agents, &router),
            ChiefPlan::Undecided
        );
    }

    #[test]
    fn prompt_lists_name_and_job_not_personas() {
        let mut a = agent("doc-keeper", "keeps docs tidy");
        a.body = "SECRET PERSONA TEXT".into();
        let prompt = plan_prompt("hi", &["user: earlier".into()], &[a]);
        assert!(prompt.contains("doc-keeper: keeps docs tidy"));
        assert!(prompt.contains("user: earlier"));
        assert!(!prompt.contains("SECRET PERSONA TEXT"));
    }
}
