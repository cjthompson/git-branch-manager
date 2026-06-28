use super::column::ColumnDef;
use super::filter::FilterTokenDef;
use crate::types::TagInfo;

pub struct TagsViewDef;

impl TagsViewDef {
    pub fn columns(&self) -> Vec<ColumnDef<TagInfo>> {
        vec![
            ColumnDef {
                name: "Name",
                min_width: 15,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.name.cmp(&b.name)),
            },
            ColumnDef {
                name: "Hash",
                min_width: 8,
                wide_width: None,
                hide_below_width: Some(80),
                compare: Some(|a, b| a.commit_hash.cmp(&b.commit_hash)),
            },
            super::column::age_column(),
            ColumnDef {
                name: "Message",
                min_width: 10,
                wide_width: None,
                hide_below_width: Some(100),
                compare: Some(|a, b| {
                    let msg_a = a.message.as_deref().unwrap_or("");
                    let msg_b = b.message.as_deref().unwrap_or("");
                    msg_a.cmp(msg_b)
                }),
            },
        ]
    }

    pub fn filter_tokens(&self) -> Vec<FilterTokenDef> {
        super::filter::age_tokens()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_correct_column_count() {
        let view = TagsViewDef;
        assert_eq!(view.columns().len(), 4);
    }

    #[test]
    fn name_column_is_sortable() {
        let view = TagsViewDef;
        let name_col = &view.columns()[0];
        assert!(name_col.compare.is_some());
    }

    #[test]
    fn hash_column_is_sortable() {
        let view = TagsViewDef;
        let hash_col = &view.columns()[1];
        assert!(hash_col.compare.is_some());
    }

    #[test]
    fn message_column_is_sortable() {
        let view = TagsViewDef;
        let msg_col = &view.columns()[3];
        assert!(msg_col.compare.is_some());
    }

    #[test]
    fn filter_tokens_only_age() {
        let view = TagsViewDef;
        let tokens = view.filter_tokens();
        assert_eq!(tokens.len(), 4);
        assert!(tokens.iter().all(|t| t.token.starts_with("age:")));
    }
}
