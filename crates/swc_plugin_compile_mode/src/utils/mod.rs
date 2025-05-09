use regex::Regex;
use std::collections::{HashMap, HashSet};
use swc_core::{
  common::{iter::IdentifyLast, util::take::Take, DUMMY_SP as span},
  ecma::{
    ast::*,
    atoms::Atom,
    utils::{quote_ident, quote_str},
    visit::{Visit, VisitWith},
  },
};

use self::{constants::*, harmony::components::get_text_component_str};
use crate::PluginConfig;
use crate::{transform_harmony::TransformVisitor, ComponentReplace};

pub mod constants;
pub mod harmony;

pub fn named_iter(str: String) -> impl FnMut() -> String {
  let mut count = -1;
  return move || {
    count += 1;
    format!("{str}{count}")
  };
}

pub fn jsx_text_to_string(atom: &Atom) -> String {
  let content = atom.replace("\t", " ");

  let res = content.lines().enumerate().identify_last().fold(
    String::new(),
    |mut acc, (is_last, (index, line))| {
      // 首行不 trim 头
      let line = if index == 0 { line } else { line.trim_start() };

      // 尾行不 trim 尾
      let line = if is_last { line } else { line.trim_end() };

      if !acc.is_empty() && !line.is_empty() {
        acc.push(' ');
      }

      acc.push_str(line);
      acc
    },
  );
  res
}

// 将驼峰写法转换为 kebab-case，即 aBcD -> a-bc-d
pub fn to_kebab_case(val: &str) -> String {
  let mut res = String::new();
  val.chars().enumerate().for_each(|(idx, c)| {
    if idx != 0 && c.is_uppercase() {
      res.push('-');
    }
    res.push(c.to_ascii_lowercase());
  });
  res
}

pub fn convert_jsx_attr_key(jsx_key: &str, adapter: &HashMap<String, String>) -> String {
  if jsx_key == "className" {
    return String::from("class");
  } else if jsx_key == COMPILE_IF
    || jsx_key == COMPILE_ELSE
    || jsx_key == COMPILE_FOR
    || jsx_key == COMPILE_FOR_KEY
  {
    let expr = match jsx_key {
      COMPILE_IF => "if",
      COMPILE_ELSE => "else",
      COMPILE_FOR => "for",
      COMPILE_FOR_KEY => "key",
      _ => "",
    };
    let adapter = adapter
      .get(expr)
      .expect(&format!("[compile mode] 模板 {} 语法未配置", expr));
    return adapter.clone();
  }
  to_kebab_case(jsx_key)
}

pub fn check_is_event_attr(val: &str) -> bool {
  val.starts_with("on") && val.chars().nth(2).is_some_and(|x| x.is_uppercase())
}

pub fn identify_jsx_event_key(val: &str, platform: &str) -> Option<String> {
  // 处理worklet事件及callback
  // 事件：     onScrollUpdateWorklet         ->  worklet:onscrollupdate
  // callback：shouldResponseOnMoveWorklet   ->  worklet:should-response-on-move
  if val.ends_with("Worklet") {
    let worklet_name = val.trim_end_matches("Worklet");
    if worklet_name.starts_with("on") {
      return Some(format!("worklet:{}", worklet_name.to_lowercase()));
    } else {
      return Some(format!("worklet:{}", to_kebab_case(worklet_name)));
    }
  }

  if check_is_event_attr(val) {
    let event_name = val.get(2..).unwrap().to_lowercase();
    let event_name = if event_name == "click" {
      "tap"
    } else {
      &event_name
    };
    let event_binding_name = match platform {
      "ALIPAY" => {
        if event_name == "tap" {
          String::from("onTap")
        } else {
          String::from(val)
        }
      }
      _ => {
        format!("bind{}", event_name)
      }
    };
    Some(event_binding_name)
  } else {
    return None;
  }
}

pub fn is_inner_component(el: &JSXElement, config: &PluginConfig) -> bool {
  let opening = &el.opening;
  if let JSXElementName::Ident(Ident { sym, .. }) = &opening.name {
    let name = to_kebab_case(&sym);
    return config.components.get(&name).is_some();
  }

  false
}

pub fn is_static_jsx(el: &Box<JSXElement>) -> bool {
  if el.opening.attrs.len() > 0 {
    return false;
  }

  for child in &el.children {
    if let JSXElementChild::JSXText(_) = child {
    } else {
      return false;
    }
  }

  true
}

pub fn create_self_closing_jsx_element_expr(
  name: JSXElementName,
  attrs: Option<Vec<JSXAttrOrSpread>>,
) -> Expr {
  Expr::JSXElement(Box::new(JSXElement {
    span,
    opening: JSXOpeningElement {
      name,
      span,
      attrs: attrs.unwrap_or(vec![]),
      self_closing: true,
      type_args: None,
    },
    children: vec![],
    closing: None,
  }))
}

pub fn create_jsx_expr_attr(name: &str, expr: Box<Expr>) -> JSXAttrOrSpread {
  JSXAttrOrSpread::JSXAttr(JSXAttr {
    span,
    name: JSXAttrName::Ident(Ident::new(name.into(), span)),
    value: Some(JSXAttrValue::JSXExprContainer(JSXExprContainer {
      span,
      expr: JSXExpr::Expr(expr),
    })),
  })
}

pub fn create_jsx_bool_attr(name: &str) -> JSXAttrOrSpread {
  JSXAttrOrSpread::JSXAttr(JSXAttr {
    span,
    name: JSXAttrName::Ident(Ident::new(name.into(), span)),
    value: None,
  })
}

pub fn create_jsx_lit_attr(name: &str, lit: Lit) -> JSXAttrOrSpread {
  JSXAttrOrSpread::JSXAttr(JSXAttr {
    span,
    name: JSXAttrName::Ident(Ident::new(name.into(), span)),
    value: Some(JSXAttrValue::Lit(lit)),
  })
}

pub fn create_jsx_dynamic_id(el: &mut JSXElement, visitor: &mut TransformVisitor) -> String {
  let node_name = (visitor.get_node_name)();

  visitor.node_name_vec.push(node_name.clone());
  el.opening
    .attrs
    .push(create_jsx_lit_attr(DYNAMIC_ID, node_name.clone().into()));
  node_name
}

pub fn add_spaces_to_lines_with_count(input: &str, count: usize) -> String {
  let mut result = String::new();

  for line in input.lines() {
    let spaces = " ".repeat(count);
    result.push_str(&format!("{}{}\n", spaces, line));
  }

  result
}

pub fn add_spaces_to_lines(input: &str) -> String {
  let count = 2;

  add_spaces_to_lines_with_count(input, count)
}

pub fn get_harmony_replace_component_dependency_define(visitor: &mut TransformVisitor) -> String {
  let component_set = &visitor.component_set;
  let component_replace = &visitor.config.component_replace;
  let mut harmony_component_style = String::new();

  component_replace.iter().for_each(|(k, v)| {
    if component_set.contains(k) {
      let ComponentReplace {
        dependency_define, ..
      } = v;

      harmony_component_style.push_str(dependency_define);
      harmony_component_style.push_str("\n");
    }
  });

  harmony_component_style
}

pub fn get_harmony_component_style(visitor: &mut TransformVisitor) -> String {
  let component_set = &visitor.component_set;
  let component_replace = &visitor.config.component_replace;
  let mut harmony_component_style = String::new();

  let mut build_component = |component_tag: &str, component_style: &str| {
    if component_set.contains(component_tag) && !component_replace.contains_key(component_tag) {
      harmony_component_style.push_str(component_style);
    }
  };

  // build_component(IMAGE_TAG, HARMONY_IMAGE_STYLE_BIND);
  // build_component(TEXT_TAG, HARMONY_TEXT_STYLE_BIND);
  build_component(TEXT_TAG, HARMONY_TEXT_BUILDER);
  build_component(TEXT_TAG, HARMONY_TEXT_HELPER_FUNCITON);

  harmony_component_style
}

pub fn check_jsx_element_has_compile_ignore(el: &JSXElement) -> bool {
  for attr in &el.opening.attrs {
    if let JSXAttrOrSpread::JSXAttr(JSXAttr { name, .. }) = attr {
      if let JSXAttrName::Ident(Ident { sym, .. }) = name {
        if sym == COMPILE_IGNORE {
          return true;
        }
      }
    }
  }
  false
}

/**
 * identify: `xx.map(function () {})` or `xx.map(() => {})`
 */
pub fn is_call_expr_of_loop(callee_expr: &mut Box<Expr>, args: &mut Vec<ExprOrSpread>) -> bool {
  if let Expr::Member(MemberExpr {
    prop: MemberProp::Ident(Ident { sym, .. }),
    ..
  }) = &mut **callee_expr
  {
    if sym == "map" {
      if let Some(ExprOrSpread { expr, .. }) = args.get_mut(0) {
        return expr.is_arrow() || expr.is_fn_expr();
      }
    }
  }
  return false;
}

pub fn is_render_fn(callee_expr: &mut Box<Expr>) -> bool {
  fn is_starts_with_render(name: &str) -> bool {
    name.starts_with("render")
  }
  match &**callee_expr {
    Expr::Member(MemberExpr {
      prop: MemberProp::Ident(Ident { sym: name, .. }),
      ..
    }) => is_starts_with_render(name),
    Expr::Ident(Ident { sym: name, .. }) => is_starts_with_render(name),
    _ => false,
  }
}

pub fn extract_jsx_loop<'a>(
  callee_expr: &mut Box<Expr>,
  args: &'a mut Vec<ExprOrSpread>,
) -> Option<&'a mut Box<JSXElement>> {
  if is_call_expr_of_loop(callee_expr, args) {
    if let Some(ExprOrSpread { expr, .. }) = args.get_mut(0) {
      fn update_return_el(return_value: &mut Box<Expr>) -> Option<&mut Box<JSXElement>> {
        if let Expr::Paren(ParenExpr { expr, .. }) = &mut **return_value {
          *return_value = expr.take();
        }
        if return_value.is_jsx_element() {
          let el = return_value.as_mut_jsx_element().unwrap();
          el.opening.attrs.push(create_jsx_bool_attr(COMPILE_FOR));
          el.opening.attrs.push(create_jsx_lit_attr(
            COMPILE_FOR_KEY,
            Lit::Str(quote_str!("sid")),
          ));
          return Some(el);
        } else if return_value.is_jsx_fragment() {
          let el = return_value.as_mut_jsx_fragment().unwrap();
          let children = el.children.take();
          let block_el = Box::new(JSXElement {
            span,
            opening: JSXOpeningElement {
              name: JSXElementName::Ident(quote_ident!("block")),
              span,
              attrs: vec![
                create_jsx_bool_attr(COMPILE_FOR),
                create_jsx_lit_attr(COMPILE_FOR_KEY, Lit::Str(quote_str!("sid"))),
              ],
              self_closing: false,
              type_args: None,
            },
            children,
            closing: Some(JSXClosingElement {
              span,
              name: JSXElementName::Ident(quote_ident!("block")),
            }),
          });
          **return_value = Expr::JSXElement(block_el);
          return Some(return_value.as_mut_jsx_element().unwrap());
        }
        None
      }
      match &mut **expr {
        Expr::Fn(FnExpr { function, .. }) => {
          if let Function {
            body: Some(BlockStmt { stmts, .. }),
            ..
          } = &mut **function
          {
            if let Some(Stmt::Return(ReturnStmt {
              arg: Some(return_value),
              ..
            })) = stmts.last_mut()
            {
              return update_return_el(return_value);
            }
          }
        }
        Expr::Arrow(ArrowExpr { body, .. }) => match &mut **body {
          BlockStmtOrExpr::BlockStmt(BlockStmt { stmts, .. }) => {
            if let Some(Stmt::Return(ReturnStmt {
              arg: Some(return_value),
              ..
            })) = stmts.last_mut()
            {
              return update_return_el(return_value);
            }
          }
          BlockStmtOrExpr::Expr(return_value) => {
            return update_return_el(return_value);
          }
        },
        _ => (),
      }
    }
  }
  None
}

pub fn get_valid_nodes(children: &Vec<JSXElementChild>) -> usize {
  let re = Regex::new(r"^\s*$").unwrap();
  let filtered_children: Vec<&JSXElementChild> = children
    .iter()
    .filter(|&item| {
      match item {
        JSXElementChild::JSXText(JSXText { value, .. }) => {
          // 用正则判断value是否只含在\n和空格，如果时，返回false
          !re.is_match(value)
        }
        _ => true,
      }
    })
    .collect();
  filtered_children.len()
}

pub fn check_jsx_element_children_exist_loop(el: &mut JSXElement) -> bool {
  for child in el.children.iter_mut() {
    if check_jsx_element_child_is_loop(child) {
      return true;
    }
  }

  false
}

pub fn check_jsx_element_child_is_loop(child: &mut JSXElementChild) -> bool {
  if let JSXElementChild::JSXExprContainer(JSXExprContainer {
    expr: JSXExpr::Expr(expr),
    ..
  }) = child
  {
    if let Expr::Call(CallExpr {
      callee: Callee::Expr(callee_expr),
      args,
      ..
    }) = &mut **expr
    {
      if is_call_expr_of_loop(callee_expr, args) {
        return true;
      }
    }
  }
  false
}

pub fn create_original_node_renderer_foreach(visitor: &mut TransformVisitor) -> String {
  add_spaces_to_lines(
    format!(
      "createLazyChildren({})",
      visitor.get_dynmaic_node_name(visitor.get_current_node_path())
    )
    .as_str(),
  )
}

pub fn create_original_node_renderer(visitor: &mut TransformVisitor) -> String {
  add_spaces_to_lines(
    format!(
      "createChildItem({} as TaroElement, createLazyChildren)",
      visitor.get_dynmaic_node_name(visitor.get_current_node_path())
    )
    .as_str(),
  )
}

pub fn create_normal_text_template(visitor: &mut TransformVisitor, disable_this: bool) -> String {
  let node_path = visitor.get_current_node_path();

  let node_name = if disable_this {
    String::from("item")
  } else {
    visitor.get_dynmaic_node_name(node_path)
  };

  let code = add_spaces_to_lines(get_text_component_str(&node_name).as_str());

  visitor.component_set.insert(TEXT_TAG.to_string());
  code
}

pub fn is_static_jsx_element_child(jsx_element: &JSXElementChild) -> bool {
  struct Visitor {
    has_jsx_expr: bool,
  }
  impl Visitor {
    fn new() -> Self {
      Visitor {
        has_jsx_expr: false,
      }
    }
  }
  impl Visit for Visitor {
    fn visit_jsx_expr_container(&mut self, _n: &JSXExprContainer) {
      self.has_jsx_expr = true;
    }
  }
  let mut visitor = Visitor::new();
  jsx_element.visit_with(&mut visitor);
  return !visitor.has_jsx_expr;
}

pub fn gen_template(val: &str) -> String {
  format!("{{{{{}}}}}", val)
}

pub fn gen_template_v(node_path: &str) -> String {
  format!("{{{{{}.v}}}}", node_path)
}

pub fn is_xscript(name: &str) -> bool {
  return name == SCRIPT_TAG;
}

pub fn as_xscript_expr_string(
  member: &MemberExpr,
  xs_module_names: &Vec<String>,
) -> Option<String> {
  if !member.prop.is_ident() {
    return None;
  }
  let prop = member.prop.as_ident().unwrap().sym.to_string();

  match &*member.obj {
    Expr::Member(lhs) => {
      let res = as_xscript_expr_string(lhs, xs_module_names);
      if res.is_some() {
        let obj = res.unwrap();
        return Some(format!("{}.{}", obj, prop));
      }
    }
    Expr::Ident(ident) => {
      let obj = ident.sym.to_string();
      if xs_module_names.contains(&obj) {
        return Some(format!("{}.{}", obj, prop));
      }
    }
    _ => (),
  }

  return None;
}

fn create_jsx_ident_opening_element(name: &str, attrs: Vec<JSXAttrOrSpread>) -> JSXOpeningElement {
  JSXOpeningElement {
    name: JSXElementName::Ident(quote_ident!(name)),
    span,
    attrs,
    self_closing: false,
    type_args: None,
  }
}

fn create_jsx_ident_closing_element(name: &str) -> JSXClosingElement {
  JSXClosingElement {
    span,
    name: JSXElementName::Ident(quote_ident!(name)),
  }
}

fn create_jsx_element(
  name: &str,
  attrs: Vec<JSXAttrOrSpread>,
  children: Vec<JSXElementChild>,
) -> JSXElement {
  JSXElement {
    span,
    opening: create_jsx_ident_opening_element(name, attrs),
    children,
    closing: Some(create_jsx_ident_closing_element(name)),
  }
}

fn extract_list_props(
  el: &mut JSXElement,
  // 需要提取的属性字段
  target_attrs: HashSet<&str>,
  // 属性别名
  attrs_alias: HashMap<&str, &str>,
) -> Vec<JSXAttrOrSpread> {
  let mut attrs = el.opening.attrs.clone();
  attrs.retain(|attr| {
    if let JSXAttrOrSpread::JSXAttr(jsx_attr) = attr {
      if let JSXAttrName::Ident(Ident { sym: name, .. }) = &jsx_attr.name {
        let attr_name = name.to_string();
        return target_attrs.contains(attr_name.as_str());
      }
    }
    false
  });

  // 根据 attrs_alias 原地修改属性名
  attrs.iter_mut().for_each(|attr| {
    if let JSXAttrOrSpread::JSXAttr(jsx_attr) = attr {
      if let JSXAttrName::Ident(Ident { sym: name, .. }) = &jsx_attr.name {
        let attr_name = name.as_str();
        if let Some(alias) = attrs_alias.get(attr_name) {
          // 如果在别名映射中找到了对应的别名，修改属性名
          jsx_attr.name = JSXAttrName::Ident(quote_ident!(*alias));
        }
      }
    }
  });

  attrs
}

fn extract_scroll_view_props(el: &mut JSXElement) -> Vec<JSXAttrOrSpread> {
  let props_alias = HashMap::from([
    ("upperThresholdCount", "upperThreshold"),
    ("lowerThresholdCount", "lowerThreshold"),
  ]);
  let target_attrs = HashSet::from([
    "scrollX",
    "scrollY",
    "scrollTop",
    "upperThresholdCount",
    "lowerThresholdCount",
    "scrollIntoView",
    "enableBackToTop",
    "showScrollbar",
    "onScroll",
    "onScrollStart",
    "onScrollEnd",
    "onScrollToUpper",
    "onScrollToLower",
    "compileMode",
    "className",
    "cacheExtent",
    "style",
    "id",
    "key",
  ]);
  let mut attrs = extract_list_props(el, target_attrs, props_alias);
  attrs.push(JSXAttrOrSpread::JSXAttr(JSXAttr {
    span,
    name: JSXAttrName::Ident(quote_ident!("type")),
    value: Some(JSXAttrValue::Lit(Lit::Str(quote_str!("custom")))),
  }));
  attrs
}

fn extract_list_builder_props(el: &mut JSXElement) -> Vec<JSXAttrOrSpread> {
  let props_alias: HashMap<&str, &str> = HashMap::from([]);
  let target_attrs = HashSet::from([
    "padding",
    "type",
    "list",
    "childCount",
    "childHeight",
    "onItemBuild",
    "onItemDispose",
  ]);
  let mut attrs = extract_list_props(el, target_attrs, props_alias);
  attrs.push(JSXAttrOrSpread::JSXAttr(JSXAttr {
    span,
    name: JSXAttrName::Ident(quote_ident!("className")),
    value: Some(JSXAttrValue::Lit(Lit::Str(quote_str!("list-builder")))),
  }));
  attrs
}

pub fn transform_list_component(el: &mut JSXElement) -> () {
  let children = el.children.clone();
  *el = create_jsx_element(
    "scroll-view",
    extract_scroll_view_props(el),
    vec![JSXElementChild::JSXElement(Box::new(create_jsx_element(
      "list-builder",
      extract_list_builder_props(el),
      children,
    )))],
  )
}

pub fn transform_list_item_component(el: &mut JSXElement) -> () {
  let children = el.children.clone();
  let mut attrs = el.opening.attrs.clone();
  attrs.push(create_jsx_lit_attr(SLOT_ITEM, "item".into()));
  attrs.push(create_jsx_lit_attr("className", "list-item".into()));
  *el = create_jsx_element("view", attrs, children)
}

pub fn transform_taro_components(
  el: &mut JSXElement,
  // 导出名和模块标识符映射关系
  import_specifiers: &HashMap<String, String>,
  // 导出名和别名映射关系
  import_aliases: &HashMap<String, String>,
) {
  match &el.clone().opening.name {
    JSXElementName::Ident(ident) => {
      if let Some(import) = import_aliases.get("List") {
        if ident.sym.as_str() == import {
          // 检查导出模块来源
          if let Some(src) = import_specifiers.get(import) {
            // 如果是 @tarojs/components 导出的 List 组件，需要特殊处理
            if src == "@tarojs/components" {
              transform_list_component(el);
            }
          }
        }
      }

      if let Some(import) = import_aliases.get("ListItem") {
        if ident.sym.as_str() == import {
          // 检查导出模块来源
          if let Some(src) = import_specifiers.get(import) {
            // 如果是 @tarojs/components 导出的 ListItem 组件，需要特殊处理
            if src == "@tarojs/components" {
              transform_list_item_component(el);
            }
          }
        }
      }
    }
    _ => (),
  };
}

#[test]
fn test_jsx_text() {
  assert_eq!("   span  ", jsx_text_to_string(&"   span  ".into()));
  assert_eq!(
    "   s xxx   ",
    jsx_text_to_string(&"   s\n     \n  \n   xxx   ".into())
  );
  assert_eq!(
    "a   b",
    jsx_text_to_string(&"   \n    \n   a   b  \n   ".into())
  );
  assert_eq!(
    "",
    jsx_text_to_string(&"\n\n\n\n\n\n                   \n\n                ".into())
  );
  assert_eq!("", jsx_text_to_string(&"".into()));
}
