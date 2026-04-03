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
                hide_below_width: None,
                compare: Some(|a, b| a.name.cmp(&b.name)),
            },
            ColumnDef {
                name: "Hash",
                min_width: 8,
                hide_below_width: Some(80),
                compare: None,
            },
            ColumnDef {
                name: "Age",
                min_width: 5,
                hide_below_width: Some(60),
                compare: Some(|a, b| a.date.cmp(&b.date)),
            },
            ColumnDef {
                name: "Message",
                min_width: 10,
                hide_below_width: Some(100),
                compare: None,
            },
        ]
    }

    pub fn filter_tokens(&self) -> Vec<FilterTokenDef> {
        vec![
            FilterTokenDef {
                key: '1',
                label: "<7 days",
                token: "age:<7d",
            },
            FilterTokenDef {
                key: '2',
                label: "<30 days",
                token: "age:<30d",
            },
            FilterTokenDef {
                key: '3',
                label: ">30 days",
                token: "age:>30d",
            },
            FilterTokenDef {
                key: '4',
                label: ">90 days",
                token: "age:>90d",
            },
        ]
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
    fn hash_column_is_not_sortable() {
        let view = TagsViewDef;
        let hash_col = &view.columns()[1];
        assert!(hash_col.compare.is_none());
    }

    #[test]
    fn message_column_is_not_sortable() {
        let view = TagsViewDef;
        let msg_col = &view.columns()[3];
        assert!(msg_col.compare.is_none());
    }

    #[test]
    fn filter_tokens_only_age() {
        let view = TagsViewDef;
        let tokens = view.filter_tokens();
        assert_eq!(tokens.len(), 4);
        assert!(tokens.iter().all(|t| t.token.starts_with("age:")));
    }
}
