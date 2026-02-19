use dashmap::DashMap;
use ropey::Rope;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::config::{
    apply_setting, get_global_config, get_global_regex, init_global_config, Settings,
};
use crate::consts::NT_RE;
use crate::input::{Input, InputResult, InputState};
use crate::rime::{Candidate, Rime, RimeError, RimeResponse};
use crate::utils::{self, Encoding};
use crate::watcher;

pub struct Backend {
    client: Client,
    documents: DashMap<String, Rope>,
    state: DashMap<String, Option<InputState>>,
    encoding: RwLock<Encoding>,
}

impl Backend {
    pub fn new(client: Client) -> Backend {
        Backend {
            client,
            documents: DashMap::new(),
            state: DashMap::new(),
            encoding: RwLock::new(Encoding::default()),
        }
    }

    async fn init(&self) -> std::result::Result<(), RimeError> {
        let (shared_data_dir, user_data_dir, log_dir) = {
            let config_arc = get_global_config();
            let config = config_arc.read().unwrap();
            crate::logger::init(utils::expand_tilde(&config.log_dir));
            let shared = utils::expand_tilde(&config.shared_data_dir)
                .to_str()
                .unwrap()
                .to_string();
            let user = utils::expand_tilde(&config.user_data_dir)
                .to_str()
                .unwrap()
                .to_string();
            let log = utils::expand_tilde(&config.log_dir)
                .to_str()
                .unwrap()
                .to_string();
            (shared, user, log)
        };
        match Rime::init(&shared_data_dir, &user_data_dir, &log_dir) {
            Err(RimeError::AlreadyInitialized) => {
                let info = "Use an initialized rime instance.";
                self.client.log_message(MessageType::INFO, info).await;
                Ok(())
            }
            Ok(()) => {
                watcher::spawn_config_watcher(&user_data_dir, crate::config::server_config_path());
                Ok(())
            }
            r => r,
        }
    }

    async fn init_config(&self, init_options: Option<Value>) {
        init_global_config(init_options);
    }

    async fn apply_settings(&self, params: Value) {
        let settings = match serde_json::from_value::<Settings>(params) {
            Ok(s) => s,
            Err(e) => {
                self.client.log_message(MessageType::ERROR, &e).await;
                self.client.show_message(MessageType::ERROR, e).await;
                return;
            }
        };

        let trigger_changed = settings.trigger_characters.is_some();
        let config = get_global_config();
        let mut cfg = config.write().unwrap();
        apply_setting!(cfg <- settings.enabled);
        apply_setting!(cfg <- settings.max_candidates);
        apply_setting!(cfg <- settings.paging_characters);
        apply_setting!(cfg <- settings.trigger_characters);
        apply_setting!(cfg <- settings.schema_trigger_character);
        apply_setting!(cfg <- settings.max_tokens);
        apply_setting!(cfg <- settings.always_incomplete);
        apply_setting!(cfg <- settings.preselect_first);
        apply_setting!(cfg <- settings.long_filter_text);
        apply_setting!(cfg <- settings.show_order_in_label);
        apply_setting!(cfg <- settings.show_comment);
        apply_setting!(cfg <- settings.hide_paging_characters);
        apply_setting!(cfg <- settings.auto_commit_on_select);
        apply_setting!(cfg <- settings.prefer_english_match);
        apply_setting!(cfg <- settings.document_dict);
        apply_setting!(cfg <- settings.document_dict_max_candidates);
        apply_setting!(cfg <- settings.document_dict_fuzzy_n_ng);
        let trigger_chars = cfg.trigger_characters.clone();
        drop(cfg);
        if trigger_changed {
            let regex = get_global_regex();
            *regex.write().unwrap() = crate::config::compile_regex_from_trigger_chars(&trigger_chars);
        }
    }

    async fn create_work_done_progress(&self, token: NumberOrString) -> Result<NumberOrString> {
        if let Err(e) = self
            .client
            .send_request::<request::WorkDoneProgressCreate>(WorkDoneProgressCreateParams {
                token: token.clone(),
            })
            .await
        {
            self.client.log_message(MessageType::WARNING, e).await;
            return Err(tower_lsp::jsonrpc::Error::internal_error());
        }
        Ok(token)
    }

    async fn notify_work_begin(&self, token: NumberOrString, message: &str) {
        // begin
        self.client
            .send_notification::<notification::Progress>(ProgressParams {
                token,
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                    WorkDoneProgressBegin {
                        title: message.to_string(),
                        ..Default::default()
                    },
                )),
            })
            .await;
    }

    async fn notify_work_done(&self, token: NumberOrString, message: &str) {
        self.client
            .send_notification::<notification::Progress>(ProgressParams {
                token,
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(WorkDoneProgressEnd {
                    message: Some(message.to_string()),
                })),
            })
            .await;
    }

    async fn get_completions(
        &self,
        uri: Url,
        position: Position,
        _context: Option<CompletionContext>,
    ) -> Option<CompletionList> {
        // get new input
        let rope = self.documents.get(uri.as_str())?;
        let encoding = *self.encoding.read().await;
        let line_begin = {
            let line_pos = Position::new(position.line, 0);
            utils::position_to_offset(&rope, line_pos, encoding)?
        };
        let curr_char = utils::position_to_offset(&rope, position, encoding)?;
        let new_input = {
            let config_arc = get_global_config();
            let regex_arc = get_global_regex();
            let config = config_arc.read().unwrap();
            let re = regex_arc.read().unwrap();
            let has_trigger = !config.trigger_characters.is_empty();
            let schema_trigger = config.schema_trigger_character.clone();
            (curr_char <= rope.len_chars())
                .then(|| {
                    let slice = Cow::from(rope.slice(line_begin..curr_char));
                    if utils::need_to_check_trigger(has_trigger, &slice) {
                        Input::new(re.as_ref(), &slice, &schema_trigger)
                    } else {
                        Input::new(&NT_RE, &slice, &schema_trigger)
                    }
                })
                .flatten()?
        };
        let new_offset = curr_char - new_input.raw_text().len();

        // handle new input
        let mut last_state = self.state.entry(uri.clone().into()).or_default();
        let had_state = (*last_state).is_some();

        // If we don't have an active composing session yet, don't treat trailing
        // digits / paging characters as "selection". This avoids interfering with
        // common ASCII identifiers like `var1` in code when rime-ls is globally enabled.
        //
        // Once a session exists, selection keys work as expected.
        if !had_state && new_input.is_selecting() {
            return None;
        }

        // 文档词在前时，数字选词需考虑偏移：先只用拼音取 Rime 候选，再补文档词，最后按用户数字决定 commit 文档词或发送正确序号给 Rime
        let (document_dict, is_first_page) = {
            let config_arc = get_global_config();
            let config = config_arc.read().unwrap();
            let paging_chars: HashSet<char> = config
                .paging_characters
                .iter()
                .filter_map(|s| s.chars().next())
                .collect();
            let is_first_page =
                !new_input.select().chars().any(|c| paging_chars.contains(&c));
            (config.document_dict, is_first_page)
        };
        let input_pinyin_only = if document_dict
            && !new_input.is_schema()
            && is_first_page
            && new_input.is_selecting()
        {
            let regex_arc = get_global_regex();
            let schema_trigger = get_global_config().read().unwrap().schema_trigger_character.clone();
            Input::new(
                regex_arc.read().unwrap().as_ref(),
                new_input.pinyin(),
                &schema_trigger,
            )
        } else {
            None
        };
        let input_to_apply = input_pinyin_only.as_ref().unwrap_or(&new_input);

        let InputResult {
            session_id,
            extra_offset,
        } = match (*last_state).as_ref() {
            Some(state) => {
                let max_tokens = get_global_config().read().unwrap().max_tokens;
                state.apply_input(new_offset, input_to_apply, max_tokens)
            }
            None => InputState::first_input(input_to_apply),
        };

        // NOTE: prevent deleting puncts before real pinyin input
        //       to achieve this, puncts in rime schema should be committed directly
        let real_offset = new_offset + extra_offset;

        let start_position = utils::offset_to_position(&rope, real_offset, encoding)?;
        let filter_prefix = get_global_config().read().unwrap().long_filter_text.then_some({
            let slice = &Cow::from(rope.slice(line_begin..real_offset));
            utils::surrounding_word(slice).to_string()
        });
        // TODO: Does compiler know the right time to drop the lock,
        // or it will wait until the end of this function?
        drop(rope);

        // get candidates from current session
        let rime = Rime::global();
        let RimeResponse {
            mut is_incomplete,
            submitted,
            mut candidates,
        } = match rime.get_response_from_session(session_id) {
            Ok(r) => r,
            Err(e) => {
                self.client.log_message(MessageType::ERROR, &e).await;
                self.client.show_message(MessageType::ERROR, e).await;
                None?
            }
        };

        // 文档临时词库：对当前文件分词，将匹配的词作为候选项补充（仅第一页显示）
        let mut num_doc_words = 0;
        {
            let config_arc = get_global_config();
            let config = config_arc.read().unwrap();
            let doc_dict = config.document_dict;
            let max_extra = config.document_dict_max_candidates;
            let fuzzy_n_ng = config.document_dict_fuzzy_n_ng;
            let paging_chars: std::collections::HashSet<char> = config
                .paging_characters
                .iter()
                .filter_map(|s| s.chars().next())
                .collect();
            let first_page = !new_input.select().chars().any(|c| paging_chars.contains(&c));
            // is_incomplete：Rime 未 commit 时才补充文档词；commit 时（数字选词后）不补充，否则会破坏自动上屏（candidates.len()==1）
            if doc_dict && !new_input.is_schema() && first_page && is_incomplete {
                if let Some(rope) = self.documents.get(uri.as_str()) {
                    let doc_text = rope.to_string();
                    let existing: HashSet<String> =
                        candidates.iter().map(|c| c.text.clone()).collect();
                    let doc_words = crate::document_dict::get_document_candidates(
                        &doc_text,
                        new_input.pinyin(),
                        &existing,
                        max_extra,
                        fuzzy_n_ng,
                    );
                    num_doc_words = doc_words.len();
                    for (i, text) in doc_words.into_iter().enumerate() {
                        candidates.insert(
                            i,
                            Candidate::from_text_with_order(text, 0), // order 0 使文档词优先
                        );
                    }
                }
            }
        }

        // When pinyin exactly matches an English word candidate, put it first
        let prefer_english_match = get_global_config().read().unwrap().prefer_english_match;
        if prefer_english_match {
            let pinyin = new_input.pinyin();
            let is_ascii_word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic());
            if is_ascii_word(pinyin) {
                if let Some(pos) = candidates
                    .iter()
                    .position(|c| c.text == pinyin && is_ascii_word(&c.text))
                {
                    if pos > 0 {
                        let candidate = candidates.remove(pos);
                        candidates.insert(0, candidate);
                    }
                }
            }
        }

        let is_selecting = new_input.is_selecting();
        let select = new_input.select().to_string();
        let hide_paging_characters = get_global_config().read().unwrap().hide_paging_characters;

        // Remove paging chars (-=,.) from document so they don't appear in text
        let (effective_range, effective_filter_text, effective_input, effective_offset) = {
            let paging_chars: std::collections::HashSet<char> = get_global_config()
                .read()
                .unwrap()
                .paging_characters
                .iter()
                .filter_map(|s| s.chars().next())
                .collect();
            let paging_suffix_len = select
                .chars()
                .rev()
                .take_while(|c| paging_chars.contains(c))
                .count();

            if hide_paging_characters && paging_suffix_len > 0 {
                let _paging_suffix: String = select
                    .chars()
                    .rev()
                    .take(paging_suffix_len)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let mut rope_guard = self.documents.get_mut(uri.as_str())?;
                let remove_start_char = curr_char - paging_suffix_len;
                let remove_range = Range::new(
                    utils::offset_to_position(&rope_guard, remove_start_char, encoding)?,
                    position.clone(),
                );
                rope_guard.remove(remove_start_char..curr_char);
                drop(rope_guard);

                const RIME_PAGING_ANNOTATION_ID: &str = "rime-paging";
                let edit = WorkspaceEdit {
                    changes: None,
                    document_changes: Some(DocumentChanges::Edits(vec![
                        TextDocumentEdit {
                            text_document: OptionalVersionedTextDocumentIdentifier {
                                uri: uri.clone(),
                                version: None,
                            },
                            edits: vec![OneOf::Right(AnnotatedTextEdit {
                                text_edit: TextEdit::new(remove_range, String::new()),
                                annotation_id: RIME_PAGING_ANNOTATION_ID.to_string(),
                            })],
                        },
                    ])),
                    change_annotations: Some(HashMap::from([(
                        RIME_PAGING_ANNOTATION_ID.to_string(),
                        ChangeAnnotation {
                            label: "Rime 翻页".to_string(),
                            needs_confirmation: Some(false),
                            description: Some("移除翻页字符，避免显示在文本中".to_string()),
                        },
                    )])),
                };
                if let Err(e) = self.client.apply_edit(edit).await {
                    self.client.log_message(MessageType::WARNING, &e.to_string()).await;
                }

                let rope = self.documents.get(uri.as_str())?;
                let effective_start = utils::offset_to_position(&rope, real_offset, encoding)?;
                let effective_end =
                    utils::offset_to_position(&rope, curr_char - paging_suffix_len, encoding)?;
                let effective_range = Range::new(effective_start, effective_end);
                let select_without_paging: String = select
                    .chars()
                    .take(select.chars().count().saturating_sub(paging_suffix_len))
                    .collect();
                let text_without_paging =
                    new_input.pinyin().to_string() + &select_without_paging;
                let effective_filter_text =
                    filter_prefix.clone().unwrap_or_default() + &text_without_paging;
                let regex_arc = get_global_regex();
                let schema_trigger = get_global_config().read().unwrap().schema_trigger_character.clone();
                let effective_input = Input::new(
                    regex_arc.read().unwrap().as_ref(),
                    &text_without_paging,
                    &schema_trigger,
                )
                .expect("text without paging should match input regex");
                let effective_offset = new_offset;
                (effective_range, effective_filter_text, effective_input, effective_offset)
            } else {
                let range = Range::new(start_position, position);
                let filter_text = filter_prefix.unwrap_or_default() + new_input.raw_text();
                (range, filter_text, new_input, new_offset)
            }
        };

        // num_doc_words > 0 时用了 input_pinyin_only，未把 select 发给 Rime；若 num_doc_words == 0 则需补发
        if document_dict && is_first_page && is_selecting && num_doc_words == 0 {
            Rime::global().process_str(session_id, &select);
            if let Ok(RimeResponse {
                is_incomplete: inc,
                candidates: cand,
                ..
            }) = Rime::global().get_response_from_session(session_id)
            {
                candidates = cand;
                is_incomplete = inc;
            }
        }

        // 文档词在前时，按用户数字选词：选文档词直接 commit，选 Rime 词则发送正确序号给 Rime 再 commit
        let auto_commit_on_select = get_global_config().read().unwrap().auto_commit_on_select;
        if auto_commit_on_select
            && had_state
            && is_selecting
            && document_dict
            && is_first_page
            && num_doc_words > 0
        {
            let selected_index = if select == "0" {
                9
            } else {
                select
                    .parse::<usize>()
                    .ok()
                    .map(|n| n.saturating_sub(1))
                    .unwrap_or(10)
            };
            if selected_index < candidates.len() {
                let commit_text = candidates[selected_index].text.clone();
                let is_doc_word = selected_index < num_doc_words;

                if is_doc_word {
                    // 选中文档词：直接 commit
                    if let Some(rope_guard) = self.documents.get(uri.as_str()) {
                        if let (Some(replace_start_char), Some(replace_end_char)) = (
                            utils::position_to_offset(&*rope_guard, effective_range.start, encoding),
                            utils::position_to_offset(&*rope_guard, effective_range.end, encoding),
                        ) {
                            drop(rope_guard);
                            if let Some(mut rope_mut) = self.documents.get_mut(uri.as_str()) {
                                if replace_start_char <= replace_end_char
                                    && replace_end_char <= rope_mut.len_chars()
                                {
                                    rope_mut.remove(replace_start_char..replace_end_char);
                                    rope_mut.insert(replace_start_char, &commit_text);
                                }
                            }
                        }
                    }
                    const RIME_AUTO_COMMIT_ANNOTATION_ID: &str = "rime-auto-commit";
                    let edit = WorkspaceEdit {
                        changes: None,
                        document_changes: Some(DocumentChanges::Edits(vec![TextDocumentEdit {
                            text_document: OptionalVersionedTextDocumentIdentifier {
                                uri: uri.clone(),
                                version: None,
                            },
                            edits: vec![OneOf::Right(AnnotatedTextEdit {
                                text_edit: TextEdit::new(effective_range.clone(), commit_text),
                                annotation_id: RIME_AUTO_COMMIT_ANNOTATION_ID.to_string(),
                            })],
                        }])),
                        change_annotations: Some(HashMap::from([(
                            RIME_AUTO_COMMIT_ANNOTATION_ID.to_string(),
                            ChangeAnnotation {
                                label: "Rime 上屏".to_string(),
                                needs_confirmation: Some(false),
                                description: Some("数字选词后自动上屏，避免额外确认".to_string()),
                            },
                        )])),
                    };
                    if let Err(e) = self.client.apply_edit(edit).await {
                        self.client.log_message(MessageType::WARNING, &e.to_string()).await;
                    }
                    Rime::global().destroy_session(session_id);
                    *last_state = None;
                    drop(last_state);
                    return None;
                }

                // 选中 Rime 词：发送正确序号给 Rime 再 commit
                let rime_index = selected_index - num_doc_words;
                let rime_key = if rime_index == 9 {
                    "0".to_string()
                } else {
                    (rime_index + 1).to_string()
                };
                Rime::global().process_str(session_id, &rime_key);
                if let Ok(RimeResponse { candidates: commit_candidates, .. }) =
                    Rime::global().get_response_from_session(session_id)
                {
                    if let Some(commit_candidate) = commit_candidates.first() {
                        let commit_text = commit_candidate.text.clone();
                        if let Some(rope_guard) = self.documents.get(uri.as_str()) {
                            if let (Some(replace_start_char), Some(replace_end_char)) = (
                                utils::position_to_offset(&*rope_guard, effective_range.start, encoding),
                                utils::position_to_offset(&*rope_guard, effective_range.end, encoding),
                            ) {
                                drop(rope_guard);
                                if let Some(mut rope_mut) = self.documents.get_mut(uri.as_str()) {
                                    if replace_start_char <= replace_end_char
                                        && replace_end_char <= rope_mut.len_chars()
                                    {
                                        rope_mut.remove(replace_start_char..replace_end_char);
                                        rope_mut.insert(replace_start_char, &commit_text);
                                    }
                                }
                            }
                        }
                        const RIME_AUTO_COMMIT_ANNOTATION_ID: &str = "rime-auto-commit";
                        let edit = WorkspaceEdit {
                            changes: None,
                            document_changes: Some(DocumentChanges::Edits(vec![TextDocumentEdit {
                                text_document: OptionalVersionedTextDocumentIdentifier {
                                    uri: uri.clone(),
                                    version: None,
                                },
                                edits: vec![OneOf::Right(AnnotatedTextEdit {
                                    text_edit: TextEdit::new(effective_range.clone(), commit_text),
                                    annotation_id: RIME_AUTO_COMMIT_ANNOTATION_ID.to_string(),
                                })],
                            }])),
                            change_annotations: Some(HashMap::from([(
                                RIME_AUTO_COMMIT_ANNOTATION_ID.to_string(),
                                ChangeAnnotation {
                                    label: "Rime 上屏".to_string(),
                                    needs_confirmation: Some(false),
                                    description: Some("数字选词后自动上屏，避免额外确认".to_string()),
                                },
                            )])),
                        };
                        if let Err(e) = self.client.apply_edit(edit).await {
                            self.client.log_message(MessageType::WARNING, &e.to_string()).await;
                        }
                        Rime::global().destroy_session(session_id);
                        *last_state = None;
                        drop(last_state);
                        return None;
                    }
                }
            }
        }

        // Auto-commit: if Rime already produced a commit text (no further candidates)
        // after selection keys (typically number keys), apply the edit immediately.
        // This avoids the "type number then confirm again" limitation in some clients.
        if auto_commit_on_select
            && had_state
            && !is_incomplete
            && candidates.len() == 1
            && is_selecting
        {
            let commit_text = candidates[0].text.clone();

            // Update server-side rope immediately to stay in sync.
            // Use effective_range (not real_offset..curr_char) because hide_paging may have
            // already removed trailing chars (e.g. space) from the document.
            if let Some(rope_guard) = self.documents.get(uri.as_str()) {
                if let (Some(replace_start_char), Some(replace_end_char)) = (
                    utils::position_to_offset(&*rope_guard, effective_range.start, encoding),
                    utils::position_to_offset(&*rope_guard, effective_range.end, encoding),
                ) {
                    drop(rope_guard);
                    if let Some(mut rope_mut) = self.documents.get_mut(uri.as_str()) {
                        if replace_start_char <= replace_end_char
                            && replace_end_char <= rope_mut.len_chars()
                        {
                            rope_mut.remove(replace_start_char..replace_end_char);
                            rope_mut.insert(replace_start_char, &commit_text);
                        }
                    }
                }
            }

            const RIME_AUTO_COMMIT_ANNOTATION_ID: &str = "rime-auto-commit";
            let edit = WorkspaceEdit {
                changes: None,
                document_changes: Some(DocumentChanges::Edits(vec![TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier {
                        uri: uri.clone(),
                        version: None,
                    },
                    edits: vec![OneOf::Right(AnnotatedTextEdit {
                        text_edit: TextEdit::new(effective_range.clone(), commit_text),
                        annotation_id: RIME_AUTO_COMMIT_ANNOTATION_ID.to_string(),
                    })],
                }])),
                change_annotations: Some(HashMap::from([(
                    RIME_AUTO_COMMIT_ANNOTATION_ID.to_string(),
                    ChangeAnnotation {
                        label: "Rime 上屏".to_string(),
                        needs_confirmation: Some(false),
                        description: Some("数字选词后自动上屏，避免额外确认".to_string()),
                    },
                )])),
            };
            if let Err(e) = self.client.apply_edit(edit).await {
                self.client.log_message(MessageType::WARNING, &e.to_string()).await;
            }

            // Clear composing state and session.
            Rime::global().destroy_session(session_id);
            *last_state = None;
            drop(last_state);

            return None;
        }

        // update input state
        *last_state = Some(InputState::new(
            effective_input,
            session_id,
            effective_offset,
            is_incomplete,
        ));
        drop(last_state);

        let filter_text = effective_filter_text;
        let range = effective_range;

        // convert candidates to completions
        let (show_order_in_label, show_comment, preselect_enabled, max_candidates) = {
            let config_arc = get_global_config();
            let config = config_arc.read().unwrap();
            (
                config.show_order_in_label,
                config.show_comment,
                config.preselect_first,
                config.max_candidates,
            )
        };
        let order_to_sort_text = utils::build_order_to_sort_text(max_candidates);
        // 统一序号：文档词与 Rime 词按最终列表位置依次编号 1, 2, 3...（参考 pyim 的 page 逻辑）
        let candidate_to_completion_item = |(i, c): (usize, Candidate)| -> CompletionItem {
            let display_order = i + 1;
            let text = match is_selecting {
                true => submitted.clone() + &c.text,
                false => c.text,
            };
            let label = if show_order_in_label {
                format!("{}. {}", display_order, &text)
            } else {
                text.clone()
            };
            let label_details = (show_comment && !c.comment.is_empty())
                .then_some(CompletionItemLabelDetails {
                    detail: Some(c.comment.clone()),
                    description: None,
                });
            // Per LSP spec: commitCharacters "accept it first and then type that character".
            // Numbers 1-9,0 would produce candidate+"1" etc - wrong for Rime number selection.
            // Space would add trailing space. Use Enter for clean commit; no commit_characters.
            let commit_characters: Option<Vec<String>> = None;
            CompletionItem {
                label,
                label_details,
                preselect: (preselect_enabled && i == 0).then_some(true),
                kind: None,
                detail: show_comment.then(|| utils::option_string(c.comment)).flatten(),
                filter_text: Some(filter_text.clone()),
                sort_text: Some(order_to_sort_text(display_order)),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(range, text))),
                commit_characters,
                ..Default::default()
            }
        };

        // return completions
        let is_incomplete = get_global_config().read().unwrap().always_incomplete || is_incomplete;
        let item_iter = candidates
            .into_iter()
            .enumerate()
            .map(candidate_to_completion_item);

        Some(CompletionList {
            is_incomplete,
            items: item_iter.collect(),
        })
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // load server config from ~/.config/rime-ls/config.yaml, then merge client init_options
        self.init_config(params.initialization_options).await;
        // init rime
        if let Err(e) = self.init().await {
            self.client.log_message(MessageType::ERROR, &e).await;
            self.client.show_message(MessageType::ERROR, e).await;
            return Err(tower_lsp::jsonrpc::Error::internal_error());
        }
        // notify client
        self.client
            .log_message(MessageType::INFO, "Rime-ls Language Server initialized")
            .await;
        // set LSP triggers
        let triggers = {
            let config_arc = get_global_config();
            let config = config_arc.read().unwrap();
            let mut triggers = config.paging_characters.clone();
            triggers.extend_from_slice(&config.trigger_characters);
            triggers
        };
        // negotiate position encoding
        let encoding_options = params
            .capabilities
            .general
            .and_then(|g| g.position_encodings);
        let encoding = utils::select_encoding(encoding_options);
        *self.encoding.write().await = encoding;

        // return
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "rime-ls".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                position_encoding: Some(PositionEncodingKind::new(encoding.as_str())),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(triggers),
                    ..CompletionOptions::default()
                }),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        "rime-ls.toggle-rime".to_string(),
                        "rime-ls.sync-user-data".to_string(),
                        "rime-ls.deploy".to_string(),
                    ],
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: Some(true),
                    },
                }),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn shutdown(&self) -> Result<()> {
        // destroy rime sessions on server shutdown
        let rime = Rime::global();
        for kvref in self.state.iter() {
            if let Some(state) = kvref.value().as_ref() {
                rime.destroy_session(state.session_id());
            }
        }
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let url = params.text_document.uri.into();
        let rope = Rope::from(params.text_document.text);
        self.documents.insert(url, rope);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let encoding = *self.encoding.read().await;
        let url = params.text_document.uri;
        if let Some(mut rope) = self.documents.get_mut(url.as_str()) {
            for change in params.content_changes {
                let TextDocumentContentChangeEvent { range, text, .. } = change;
                match range {
                    // incremental change
                    Some(Range { start, end }) => {
                        let s = utils::position_to_offset(&rope, start, encoding);
                        let e = utils::position_to_offset(&rope, end, encoding);
                        if let (Some(s), Some(e)) = (s, e) {
                            rope.remove(s..e);
                            rope.insert(s, &text);
                        }
                    }
                    // full content change
                    None => {
                        *rope = Rope::from(text);
                    }
                }
            }
        }
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.client
            .log_message(MessageType::INFO, "settings changed")
            .await;
        self.apply_settings(params.settings).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.as_str();
        self.documents.remove(uri);
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        if !get_global_config().read().unwrap().enabled {
            return Ok(None);
        }
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let context = params.context;

        let completions = self.get_completions(uri, position, context).await;
        Ok(completions.map(CompletionResponse::List))
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        let command: &str = params.command.as_ref();
        let token = {
            match params.work_done_progress_params.work_done_token {
                Some(token) => token,
                None => {
                    let token = NumberOrString::String(command.to_string());
                    self.create_work_done_progress(token).await?
                }
            }
        };
        match command {
            "rime-ls.toggle-rime" => {
                self.notify_work_begin(token.clone(), command).await;
                let config_arc = get_global_config();
                let enabled = {
                    let mut config = config_arc.write().unwrap();
                    config.enabled = !config.enabled;
                    config.enabled
                };
                let status = if enabled { "Rime is ON" } else { "Rime is OFF" };
                self.notify_work_done(token.clone(), status).await;
                return Ok(Some(Value::from(enabled)));
            }
            "rime-ls.sync-user-data" => {
                self.notify_work_begin(token.clone(), command).await;
                Rime::global().sync_user_data();
                self.notify_work_done(token.clone(), "Rime is Ready.").await;
            }
            "rime-ls.deploy" => {
                self.notify_work_begin(token.clone(), command).await;
                Rime::global().deploy();
                self.notify_work_done(token.clone(), "Rime deployed.").await;
            }
            _ => {
                self.client
                    .log_message(MessageType::WARNING, "No such rime-ls command")
                    .await;
            }
        }
        Ok(None)
    }
}
