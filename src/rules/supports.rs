//! The `@supports` rule.

use super::Location;
use super::{CssRuleList, MinifyContext};
use crate::error::{MinifyError, ParserError, PrinterError};
use crate::parser::DefaultAtRule;
use crate::printer::Printer;
use crate::properties::PropertyId;
use crate::rules::{StyleContext, ToCssWithContext};
use crate::targets::Browsers;
use crate::traits::{Parse, ToCss};
use crate::values::string::CowArcStr;
use crate::vendor_prefix::VendorPrefix;
#[cfg(feature = "visitor")]
use crate::visitor::Visit;
use cssparser::*;

#[cfg(feature = "serde")]
use crate::serialization::ValueWrapper;

/// A [@supports](https://drafts.csswg.org/css-conditional-3/#at-supports) rule.
#[derive(Debug, PartialEq, Clone)]
#[cfg_attr(feature = "visitor", derive(Visit))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct SupportsRule<'i, R = DefaultAtRule> {
  /// The supports condition.
  #[cfg_attr(feature = "serde", serde(borrow))]
  pub condition: SupportsCondition<'i>,
  /// The rules within the `@supports` rule.
  pub rules: CssRuleList<'i, R>,
  /// The location of the rule in the source file.
  #[cfg_attr(feature = "visitor", skip_visit)]
  pub loc: Location,
}

impl<'i, T> SupportsRule<'i, T> {
  pub(crate) fn minify(
    &mut self,
    context: &mut MinifyContext<'_, 'i>,
    parent_is_unused: bool,
  ) -> Result<(), MinifyError> {
    if let Some(targets) = context.targets {
      self.condition.set_prefixes_for_targets(targets)
    }

    self.rules.minify(context, parent_is_unused)
  }
}

impl<'a, 'i, T: ToCss> ToCssWithContext<'a, 'i, T> for SupportsRule<'i, T> {
  fn to_css_with_context<W>(
    &self,
    dest: &mut Printer<W>,
    context: Option<&StyleContext<'a, 'i, T>>,
  ) -> Result<(), PrinterError>
  where
    W: std::fmt::Write,
  {
    #[cfg(feature = "sourcemap")]
    dest.add_mapping(self.loc);
    dest.write_str("@supports ")?;
    self.condition.to_css(dest)?;
    dest.whitespace()?;
    dest.write_char('{')?;
    dest.indent();
    dest.newline()?;
    self.rules.to_css_with_context(dest, context)?;
    dest.dedent();
    dest.newline()?;
    dest.write_char('}')
  }
}

/// A [`<supports-condition>`](https://drafts.csswg.org/css-conditional-3/#typedef-supports-condition),
/// as used in the `@supports` and `@import` rules.
#[derive(Debug, PartialEq, Clone)]
#[cfg_attr(feature = "visitor", derive(Visit))]
#[cfg_attr(feature = "into_owned", derive(lightningcss_derive::IntoOwned))]
#[cfg_attr(feature = "visitor", visit(visit_supports_condition, SUPPORTS_CONDITIONS))]
#[cfg_attr(
  feature = "serde",
  derive(serde::Serialize, serde::Deserialize),
  serde(tag = "type", rename_all = "kebab-case")
)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub enum SupportsCondition<'i> {
  /// A `not` expression.
  #[cfg_attr(feature = "visitor", skip_type)]
  #[cfg_attr(feature = "serde", serde(with = "ValueWrapper::<Box<SupportsCondition>>"))]
  Not(Box<SupportsCondition<'i>>),
  /// An `and` expression.
  #[cfg_attr(feature = "visitor", skip_type)]
  #[cfg_attr(feature = "serde", serde(with = "ValueWrapper::<Vec<SupportsCondition>>"))]
  And(Vec<SupportsCondition<'i>>),
  /// An `or` expression.
  #[cfg_attr(feature = "visitor", skip_type)]
  #[cfg_attr(feature = "serde", serde(with = "ValueWrapper::<Vec<SupportsCondition>>"))]
  Or(Vec<SupportsCondition<'i>>),
  /// A declaration to evaluate.
  Declaration {
    /// The property id for the declaration.
    #[cfg_attr(feature = "serde", serde(borrow, rename = "propertyId"))]
    property_id: PropertyId<'i>,
    /// The raw value of the declaration.
    value: CowArcStr<'i>,
  },
  /// A selector to evaluate.
  #[cfg_attr(feature = "serde", serde(with = "ValueWrapper::<CowArcStr>"))]
  Selector(CowArcStr<'i>),
  // FontTechnology()
  /// An unknown condition.
  #[cfg_attr(feature = "serde", serde(with = "ValueWrapper::<CowArcStr>"))]
  Unknown(CowArcStr<'i>),
}

impl<'i> SupportsCondition<'i> {
  /// Combines the given supports condition into this one with an `and` expression.
  pub fn and(&mut self, b: &SupportsCondition<'i>) {
    if let SupportsCondition::And(a) = self {
      if !a.contains(&b) {
        a.push(b.clone());
      }
    } else if self != b {
      *self = SupportsCondition::And(vec![self.clone(), b.clone()])
    }
  }

  /// Combines the given supports condition into this one with an `or` expression.
  pub fn or(&mut self, b: &SupportsCondition<'i>) {
    if let SupportsCondition::Or(a) = self {
      if !a.contains(&b) {
        a.push(b.clone());
      }
    } else if self != b {
      *self = SupportsCondition::Or(vec![self.clone(), b.clone()])
    }
  }

  fn set_prefixes_for_targets(&mut self, targets: &Browsers) {
    match self {
      SupportsCondition::Not(cond) => cond.set_prefixes_for_targets(targets),
      SupportsCondition::And(items) | SupportsCondition::Or(items) => {
        for item in items {
          item.set_prefixes_for_targets(targets);
        }
      }
      SupportsCondition::Declaration { property_id, .. } => {
        let prefix = property_id.prefix();
        if prefix.is_empty() || prefix.contains(VendorPrefix::None) {
          property_id.set_prefixes_for_targets(*targets);
        }
      }
      _ => {}
    }
  }
}

impl<'i> Parse<'i> for SupportsCondition<'i> {
  fn parse<'t>(input: &mut Parser<'i, 't>) -> Result<Self, ParseError<'i, ParserError<'i>>> {
    if input.try_parse(|input| input.expect_ident_matching("not")).is_ok() {
      let in_parens = Self::parse_in_parens(input)?;
      return Ok(SupportsCondition::Not(Box::new(in_parens)));
    }

    let in_parens = Self::parse_in_parens(input)?;
    let mut expected_type = None;
    let mut conditions = Vec::new();

    loop {
      let condition = input.try_parse(|input| {
        let location = input.current_source_location();
        let s = input.expect_ident()?;
        let found_type = match_ignore_ascii_case! { &s,
          "and" => 1,
          "or" => 2,
          _ => return Err(location.new_unexpected_token_error(
            cssparser::Token::Ident(s.clone())
          ))
        };

        if let Some(expected) = expected_type {
          if found_type != expected {
            return Err(location.new_unexpected_token_error(cssparser::Token::Ident(s.clone())));
          }
        } else {
          expected_type = Some(found_type);
        }

        Self::parse_in_parens(input)
      });

      if let Ok(condition) = condition {
        if conditions.is_empty() {
          conditions.push(in_parens.clone())
        }
        conditions.push(condition)
      } else {
        break;
      }
    }

    match expected_type {
      Some(1) => Ok(SupportsCondition::And(conditions)),
      Some(2) => Ok(SupportsCondition::Or(conditions)),
      _ => Ok(in_parens),
    }
  }
}

impl<'i> SupportsCondition<'i> {
  fn parse_in_parens<'t>(input: &mut Parser<'i, 't>) -> Result<Self, ParseError<'i, ParserError<'i>>> {
    input.skip_whitespace();
    let location = input.current_source_location();
    let pos = input.position();
    match input.next()? {
      Token::Function(ref f) => {
        match_ignore_ascii_case! { &*f,
          "selector" => {
            let res = input.try_parse(|input| {
              input.parse_nested_block(|input| {
                let pos = input.position();
                input.expect_no_error_token()?;
                Ok(SupportsCondition::Selector(input.slice_from(pos).into()))
              })
            });
            if res.is_ok() {
              return res
            }
          },
          _ => {}
        }
      }
      Token::ParenthesisBlock => {
        let res = input.try_parse(|input| {
          input.parse_nested_block(|input| {
            if let Ok(condition) = input.try_parse(SupportsCondition::parse) {
              return Ok(condition);
            }

            Self::parse_declaration(input)
          })
        });
        if res.is_ok() {
          return res;
        }
      }
      t => return Err(location.new_unexpected_token_error(t.clone())),
    };

    input.parse_nested_block(|input| input.expect_no_error_token().map_err(|err| err.into()))?;
    Ok(SupportsCondition::Unknown(input.slice_from(pos).into()))
  }

  pub(crate) fn parse_declaration<'t>(
    input: &mut Parser<'i, 't>,
  ) -> Result<Self, ParseError<'i, ParserError<'i>>> {
    let property_id = PropertyId::parse(input)?;
    input.expect_colon()?;
    input.skip_whitespace();
    let pos = input.position();
    input.expect_no_error_token()?;
    Ok(SupportsCondition::Declaration {
      property_id,
      value: input.slice_from(pos).into(),
    })
  }

  fn needs_parens(&self, parent: &SupportsCondition) -> bool {
    match self {
      SupportsCondition::Not(_) => true,
      SupportsCondition::And(_) => !matches!(parent, SupportsCondition::And(_)),
      SupportsCondition::Or(_) => !matches!(parent, SupportsCondition::Or(_)),
      _ => false,
    }
  }

  fn to_css_with_parens_if_needed<W>(&self, dest: &mut Printer<W>, needs_parens: bool) -> Result<(), PrinterError>
  where
    W: std::fmt::Write,
  {
    if needs_parens {
      dest.write_char('(')?;
    }
    self.to_css(dest)?;
    if needs_parens {
      dest.write_char(')')?;
    }
    Ok(())
  }
}

impl<'i> ToCss for SupportsCondition<'i> {
  fn to_css<W>(&self, dest: &mut Printer<W>) -> Result<(), PrinterError>
  where
    W: std::fmt::Write,
  {
    match self {
      SupportsCondition::Not(condition) => {
        dest.write_str("not ")?;
        condition.to_css_with_parens_if_needed(dest, condition.needs_parens(self))
      }
      SupportsCondition::And(conditions) => {
        let mut first = true;
        for condition in conditions {
          if first {
            first = false;
          } else {
            dest.write_str(" and ")?;
          }
          condition.to_css_with_parens_if_needed(dest, condition.needs_parens(self))?;
        }
        Ok(())
      }
      SupportsCondition::Or(conditions) => {
        let mut first = true;
        for condition in conditions {
          if first {
            first = false;
          } else {
            dest.write_str(" or ")?;
          }
          condition.to_css_with_parens_if_needed(dest, condition.needs_parens(self))?;
        }
        Ok(())
      }
      SupportsCondition::Declaration { property_id, value } => {
        dest.write_char('(')?;

        let prefix = property_id.prefix().or_none();
        if prefix != VendorPrefix::None {
          dest.write_char('(')?;
        }

        let name = property_id.name();
        let mut first = true;
        for p in prefix {
          if first {
            first = false;
          } else {
            dest.write_str(") or (")?;
          }

          p.to_css(dest)?;
          serialize_name(name, dest)?;
          dest.delim(':', false)?;
          dest.write_str(value)?;
        }

        if prefix != VendorPrefix::None {
          dest.write_char(')')?;
        }

        dest.write_char(')')
      }
      SupportsCondition::Selector(sel) => {
        dest.write_str("selector(")?;
        dest.write_str(sel)?;
        dest.write_char(')')
      }
      SupportsCondition::Unknown(unknown) => dest.write_str(&unknown),
    }
  }
}
