//! View SQL → IVM plan: parse the SELECT statement and extract GROUP BY
//! columns and aggregate functions.
//!
//! In v0.11 we support a subset of SQL:
//!   - GROUP BY on one or more named columns
//!   - COUNT(*), SUM(col), MIN(col), MAX(col) aggregates

use crate::worker::IvmError;

/// A single aggregate function in the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregate {
    /// Output column name (alias or auto-generated).
    pub output_col: String,
    /// Aggregate kind.
    pub kind: AggregateKind,
    /// Input column (None for COUNT(*)).
    pub input_col: Option<String>,
}

/// Supported aggregate functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregateKind {
    Count,
    Sum,
    Min,
    Max,
}

/// Parsed IVM plan extracted from view SQL.
#[derive(Debug, Clone)]
pub struct IvmPlan {
    /// The original view SQL.
    pub view_sql: String,
    /// GROUP BY column names (in order).
    pub group_by_cols: Vec<String>,
    /// Aggregate functions.
    pub aggregates: Vec<Aggregate>,
}

impl IvmPlan {
    /// Parse a view SQL string and extract the IVM plan.
    pub fn parse(view_sql: &str) -> Result<Self, IvmError> {
        use sqlparser::ast::{
            Expr, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr, SelectItem, SetExpr,
        };
        use sqlparser::dialect::GenericDialect;
        use sqlparser::parser::Parser;

        let dialect = GenericDialect {};
        let ast = Parser::parse_sql(&dialect, view_sql)
            .map_err(|e| IvmError::PlanParse(e.to_string()))?;

        let stmt = ast
            .into_iter()
            .next()
            .ok_or_else(|| IvmError::PlanParse("empty SQL".into()))?;

        let query = match stmt {
            sqlparser::ast::Statement::Query(q) => q,
            _ => return Err(IvmError::PlanParse("expected SELECT query".into())),
        };

        let select = match *query.body {
            SetExpr::Select(s) => s,
            _ => return Err(IvmError::PlanParse("expected SELECT body".into())),
        };

        // Extract GROUP BY columns.
        let group_by_cols: Vec<String> = match &select.group_by {
            GroupByExpr::Expressions(exprs, _) => exprs
                .iter()
                .filter_map(|expr| match expr {
                    Expr::Identifier(id) => Some(id.value.to_lowercase()),
                    Expr::CompoundIdentifier(parts) => parts.last().map(|p| p.value.to_lowercase()),
                    _ => None,
                })
                .collect(),
            GroupByExpr::All(_) => Vec::new(),
        };

        // Extract aggregates from the projection.
        let mut aggregates = Vec::new();
        for item in &select.projection {
            let (alias, expr) = match item {
                SelectItem::ExprWithAlias { expr, alias } => {
                    (Some(alias.value.clone()), expr.clone())
                }
                SelectItem::UnnamedExpr(e) => (None, e.clone()),
                _ => continue,
            };
            if let Expr::Function(f) = expr {
                let fn_name = f.name.to_string().to_uppercase();
                let kind = match fn_name.as_str() {
                    "COUNT" => AggregateKind::Count,
                    "SUM" => AggregateKind::Sum,
                    "MIN" => AggregateKind::Min,
                    "MAX" => AggregateKind::Max,
                    _ => continue,
                };

                let arg_list = match &f.args {
                    FunctionArguments::List(l) => l.args.as_slice(),
                    _ => &[],
                };

                let input_col = arg_list.iter().find_map(|a| match a {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Identifier(id))) => {
                        Some(id.value.to_lowercase())
                    }
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::CompoundIdentifier(
                        parts,
                    ))) => parts.last().map(|p| p.value.to_lowercase()),
                    FunctionArg::Unnamed(FunctionArgExpr::Wildcard) => None, // COUNT(*)
                    _ => None,
                });

                let output_col = alias.unwrap_or_else(|| {
                    if let Some(ref col) = input_col {
                        format!("{}_{}", fn_name.to_lowercase(), col)
                    } else {
                        fn_name.to_lowercase()
                    }
                });

                aggregates.push(Aggregate {
                    output_col,
                    kind,
                    input_col,
                });
            }
        }

        Ok(IvmPlan {
            view_sql: view_sql.to_string(),
            group_by_cols,
            aggregates,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_count_star_group_by() {
        let sql = "SELECT region, COUNT(*) AS cnt FROM sales GROUP BY region";
        let plan = IvmPlan::parse(sql).unwrap();
        assert_eq!(plan.group_by_cols, vec!["region"]);
        assert_eq!(plan.aggregates.len(), 1);
        assert_eq!(plan.aggregates[0].kind, AggregateKind::Count);
        assert_eq!(plan.aggregates[0].output_col, "cnt");
    }

    #[test]
    fn parse_sum_aggregate() {
        let sql = "SELECT dept, SUM(amount) AS total FROM orders GROUP BY dept";
        let plan = IvmPlan::parse(sql).unwrap();
        assert_eq!(plan.group_by_cols, vec!["dept"]);
        assert_eq!(plan.aggregates[0].kind, AggregateKind::Sum);
        assert_eq!(plan.aggregates[0].input_col, Some("amount".to_string()));
    }

    #[test]
    fn parse_multi_agg() {
        let sql = "SELECT dept, MIN(salary) AS lo, MAX(salary) AS hi FROM emp GROUP BY dept";
        let plan = IvmPlan::parse(sql).unwrap();
        assert_eq!(plan.aggregates.len(), 2);
        assert_eq!(plan.aggregates[0].kind, AggregateKind::Min);
        assert_eq!(plan.aggregates[1].kind, AggregateKind::Max);
    }

    #[test]
    fn invalid_sql_returns_error() {
        assert!(IvmPlan::parse("NOT SQL").is_err());
    }
}
