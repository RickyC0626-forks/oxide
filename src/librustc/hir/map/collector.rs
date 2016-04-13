// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use super::*;
use super::MapEntry::*;

use hir::*;
use hir::intravisit::Visitor;
use hir::def_id::{CRATE_DEF_INDEX, DefId, DefIndex};
use middle::cstore::InlinedItem;
use std::iter::repeat;
use syntax::ast::{NodeId, CRATE_NODE_ID, DUMMY_NODE_ID};
use syntax::codemap::Span;

/// A Visitor that walks over the HIR and collects Node's into a HIR map.
pub struct NodeCollector<'ast> {
    pub krate: &'ast Crate,
    pub map: Vec<MapEntry<'ast>>,
    pub parent_node: NodeId,
}

pub struct DefCollector<'ast> {
    pub krate: &'ast Crate,
    pub map: &'ast [MapEntry<'ast>],
    pub definitions: Definitions,
    pub parent_def: Option<DefIndex>,
}

impl<'ast> DefCollector<'ast> {
    pub fn root(krate: &'ast Crate, map: &'ast [MapEntry<'ast>]) -> DefCollector<'ast> {
        let mut collector = DefCollector {
            krate: krate,
            map: map,
            definitions: Definitions::new(),
            parent_def: None,
        };
        let result = collector.create_def_with_parent(None, CRATE_NODE_ID, DefPathData::CrateRoot);
        assert_eq!(result, CRATE_DEF_INDEX);

        collector.create_def_with_parent(Some(CRATE_DEF_INDEX), DUMMY_NODE_ID, DefPathData::Misc);

        collector
    }

    pub fn extend(krate: &'ast Crate,
                  parent_node: NodeId,
                  parent_def_path: DefPath,
                  parent_def_id: DefId,
                  map: &'ast [MapEntry<'ast>],
                  definitions: Definitions)
                  -> DefCollector<'ast> {
        let mut collector = DefCollector {
            krate: krate,
            map: map,
            parent_def: None,
            definitions: definitions,
        };

        assert_eq!(parent_def_path.krate, parent_def_id.krate);
        let root_path = Box::new(InlinedRootPath {
            data: parent_def_path.data,
            def_id: parent_def_id,
        });

        let def = collector.create_def(parent_node, DefPathData::InlinedRoot(root_path));
        collector.parent_def = Some(def);

        collector
    }

    fn parent_def(&self) -> Option<DefIndex> {
        self.parent_def
    }

    fn create_def(&mut self, node_id: NodeId, data: DefPathData) -> DefIndex {
        let parent_def = self.parent_def();
        debug!("create_def(node_id={:?}, data={:?}, parent_def={:?})", node_id, data, parent_def);
        self.definitions.create_def_with_parent(parent_def, node_id, data)
    }

    fn create_def_with_parent(&mut self,
                              parent: Option<DefIndex>,
                              node_id: NodeId,
                              data: DefPathData)
                              -> DefIndex {
        self.definitions.create_def_with_parent(parent, node_id, data)
    }
}

impl<'ast> Visitor<'ast> for DefCollector<'ast> {
    /// Because we want to track parent items and so forth, enable
    /// deep walking so that we walk nested items in the context of
    /// their outer items.
    fn visit_nested_item(&mut self, item: ItemId) {
        debug!("visit_nested_item: {:?}", item);
        self.visit_item(self.krate.item(item.id))
    }

    fn visit_item(&mut self, i: &'ast Item) {
        debug!("visit_item: {:?}", i);

        // Pick the def data. This need not be unique, but the more
        // information we encapsulate into
        let def_data = match i.node {
            ItemDefaultImpl(..) | ItemImpl(..) =>
                DefPathData::Impl,
            ItemEnum(..) | ItemStruct(..) | ItemTrait(..) |
            ItemExternCrate(..) | ItemForeignMod(..) | ItemTy(..) =>
                DefPathData::TypeNs(i.name),
            ItemMod(..) =>
                DefPathData::Module(i.name),
            ItemStatic(..) | ItemConst(..) | ItemFn(..) =>
                DefPathData::ValueNs(i.name),
            ItemUse(..) =>
                DefPathData::Misc,
        };

        let def = self.create_def(i.id, def_data);

        let parent_def = self.parent_def;
        self.parent_def = Some(def);

        match i.node {
            ItemEnum(ref enum_definition, _) => {
                for v in &enum_definition.variants {
                    let variant_def_index =
                        self.create_def(v.node.data.id(),
                                        DefPathData::EnumVariant(v.node.name));

                    for field in v.node.data.fields() {
                        self.create_def_with_parent(
                            Some(variant_def_index),
                            field.id,
                            DefPathData::Field(field.name));
                    }
                }
            }
            ItemStruct(ref struct_def, _) => {
                // If this is a tuple-like struct, register the constructor.
                if !struct_def.is_struct() {
                    self.create_def(struct_def.id(),
                                    DefPathData::StructCtor);
                }

                for field in struct_def.fields() {
                    self.create_def(field.id, DefPathData::Field(field.name));
                }
            }
            _ => {}
        }
        intravisit::walk_item(self, i);
        self.parent_def = parent_def;
    }

    fn visit_foreign_item(&mut self, foreign_item: &'ast ForeignItem) {
        let def = self.create_def(foreign_item.id, DefPathData::ValueNs(foreign_item.name));

        let parent_def = self.parent_def;
        self.parent_def = Some(def);
        intravisit::walk_foreign_item(self, foreign_item);
        self.parent_def = parent_def;
    }

    fn visit_generics(&mut self, generics: &'ast Generics) {
        for ty_param in generics.ty_params.iter() {
            self.create_def(ty_param.id,
                            DefPathData::TypeParam(ty_param.name));
        }

        intravisit::walk_generics(self, generics);
    }

    fn visit_trait_item(&mut self, ti: &'ast TraitItem) {
        let def_data = match ti.node {
            MethodTraitItem(..) | ConstTraitItem(..) => DefPathData::ValueNs(ti.name),
            TypeTraitItem(..) => DefPathData::TypeNs(ti.name),
        };

        let def = self.create_def(ti.id, def_data);

        let parent_def = self.parent_def;
        self.parent_def = Some(def);

        match ti.node {
            ConstTraitItem(_, Some(ref expr)) => {
                self.create_def(expr.id, DefPathData::Initializer);
            }
            _ => { }
        }

        intravisit::walk_trait_item(self, ti);

        self.parent_def = parent_def;
    }

    fn visit_impl_item(&mut self, ii: &'ast ImplItem) {
        let def_data = match ii.node {
            ImplItemKind::Method(..) | ImplItemKind::Const(..) => DefPathData::ValueNs(ii.name),
            ImplItemKind::Type(..) => DefPathData::TypeNs(ii.name),
        };

        let def = self.create_def(ii.id, def_data);

        let parent_def = self.parent_def;
        self.parent_def = Some(def);

        match ii.node {
            ImplItemKind::Const(_, ref expr) => {
                self.create_def(expr.id, DefPathData::Initializer);
            }
            _ => { }
        }

        intravisit::walk_impl_item(self, ii);

        self.parent_def = parent_def;
    }

    fn visit_pat(&mut self, pat: &'ast Pat) {
        let maybe_binding = match pat.node {
            PatKind::Ident(_, id, _) => Some(id.node),
            _ => None
        };

        let parent_def = self.parent_def;
        if let Some(id) = maybe_binding {
            let def = self.create_def(pat.id, DefPathData::Binding(id.name));
            self.parent_def = Some(def);
        }

        intravisit::walk_pat(self, pat);
        self.parent_def = parent_def;
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        let parent_def = self.parent_def;

        if let ExprClosure(..) = expr.node {
            let def = self.create_def(expr.id, DefPathData::ClosureExpr);
            self.parent_def = Some(def);
        }

        intravisit::walk_expr(self, expr);
        self.parent_def = parent_def;
    }

    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        intravisit::walk_stmt(self, stmt);
    }

    fn visit_block(&mut self, block: &'ast Block) {
        intravisit::walk_block(self, block);
    }

    fn visit_lifetime_def(&mut self, def: &'ast LifetimeDef) {
        self.create_def(def.lifetime.id, DefPathData::LifetimeDef(def.lifetime.name));
    }

    fn visit_macro_def(&mut self, macro_def: &'ast MacroDef) {
        self.create_def(macro_def.id, DefPathData::MacroDef(macro_def.name));
    }
}

impl<'ast> NodeCollector<'ast> {
    pub fn root(krate: &'ast Crate) -> NodeCollector<'ast> {
        let mut collector = NodeCollector {
            krate: krate,
            map: vec![],
            parent_node: CRATE_NODE_ID,
        };
        collector.insert_entry(CRATE_NODE_ID, RootCrate);

        collector
    }

    pub fn extend(krate: &'ast Crate,
                  parent: &'ast InlinedItem,
                  parent_node: NodeId,
                  parent_def_path: DefPath,
                  parent_def_id: DefId,
                  map: Vec<MapEntry<'ast>>)
                  -> NodeCollector<'ast> {
        let mut collector = NodeCollector {
            krate: krate,
            map: map,
            parent_node: parent_node,
        };

        assert_eq!(parent_def_path.krate, parent_def_id.krate);
        collector.insert_entry(parent_node, RootInlinedParent(parent));

        collector
    }

    fn insert_entry(&mut self, id: NodeId, entry: MapEntry<'ast>) {
        debug!("ast_map: {:?} => {:?}", id, entry);
        let len = self.map.len();
        if id as usize >= len {
            self.map.extend(repeat(NotPresent).take(id as usize - len + 1));
        }
        self.map[id as usize] = entry;
    }

    fn insert(&mut self, id: NodeId, node: Node<'ast>) {
        let entry = MapEntry::from_node(self.parent_node, node);
        self.insert_entry(id, entry);
    }
}

impl<'ast> Visitor<'ast> for NodeCollector<'ast> {
    /// Because we want to track parent items and so forth, enable
    /// deep walking so that we walk nested items in the context of
    /// their outer items.
    fn visit_nested_item(&mut self, item: ItemId) {
        debug!("visit_nested_item: {:?}", item);
        self.visit_item(self.krate.item(item.id))
    }

    fn visit_item(&mut self, i: &'ast Item) {
        debug!("visit_item: {:?}", i);

        self.insert(i.id, NodeItem(i));

        let parent_node = self.parent_node;
        self.parent_node = i.id;

        match i.node {
            ItemEnum(ref enum_definition, _) => {
                for v in &enum_definition.variants {
                    self.insert(v.node.data.id(), NodeVariant(v));
                }
            }
            ItemStruct(ref struct_def, _) => {
                // If this is a tuple-like struct, register the constructor.
                if !struct_def.is_struct() {
                    self.insert(struct_def.id(), NodeStructCtor(struct_def));
                }
            }
            ItemTrait(_, _, ref bounds, _) => {
                for b in bounds.iter() {
                    if let TraitTyParamBound(ref t, TraitBoundModifier::None) = *b {
                        self.insert(t.trait_ref.ref_id, NodeItem(i));
                    }
                }
            }
            ItemUse(ref view_path) => {
                match view_path.node {
                    ViewPathList(_, ref paths) => {
                        for path in paths {
                            self.insert(path.node.id(), NodeItem(i));
                        }
                    }
                    _ => ()
                }
            }
            _ => {}
        }
        intravisit::walk_item(self, i);
        self.parent_node = parent_node;
    }

    fn visit_foreign_item(&mut self, foreign_item: &'ast ForeignItem) {
        self.insert(foreign_item.id, NodeForeignItem(foreign_item));

        let parent_node = self.parent_node;
        self.parent_node = foreign_item.id;
        intravisit::walk_foreign_item(self, foreign_item);
        self.parent_node = parent_node;
    }

    fn visit_generics(&mut self, generics: &'ast Generics) {
        for ty_param in generics.ty_params.iter() {
            self.insert(ty_param.id, NodeTyParam(ty_param));
        }

        intravisit::walk_generics(self, generics);
    }

    fn visit_trait_item(&mut self, ti: &'ast TraitItem) {
        self.insert(ti.id, NodeTraitItem(ti));

        let parent_node = self.parent_node;
        self.parent_node = ti.id;

        intravisit::walk_trait_item(self, ti);

        self.parent_node = parent_node;
    }

    fn visit_impl_item(&mut self, ii: &'ast ImplItem) {
        self.insert(ii.id, NodeImplItem(ii));

        let parent_node = self.parent_node;
        self.parent_node = ii.id;

        intravisit::walk_impl_item(self, ii);

        self.parent_node = parent_node;
    }

    fn visit_pat(&mut self, pat: &'ast Pat) {
        self.insert(pat.id, NodeLocal(pat));

        let parent_node = self.parent_node;
        self.parent_node = pat.id;
        intravisit::walk_pat(self, pat);
        self.parent_node = parent_node;
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        self.insert(expr.id, NodeExpr(expr));

        let parent_node = self.parent_node;
        self.parent_node = expr.id;
        intravisit::walk_expr(self, expr);
        self.parent_node = parent_node;
    }

    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        let id = stmt.node.id();
        self.insert(id, NodeStmt(stmt));
        let parent_node = self.parent_node;
        self.parent_node = id;
        intravisit::walk_stmt(self, stmt);
        self.parent_node = parent_node;
    }

    fn visit_fn(&mut self, fk: intravisit::FnKind<'ast>, fd: &'ast FnDecl,
                b: &'ast Block, s: Span, id: NodeId) {
        assert_eq!(self.parent_node, id);
        intravisit::walk_fn(self, fk, fd, b, s);
    }

    fn visit_block(&mut self, block: &'ast Block) {
        self.insert(block.id, NodeBlock(block));
        let parent_node = self.parent_node;
        self.parent_node = block.id;
        intravisit::walk_block(self, block);
        self.parent_node = parent_node;
    }

    fn visit_lifetime(&mut self, lifetime: &'ast Lifetime) {
        self.insert(lifetime.id, NodeLifetime(lifetime));
    }
}
