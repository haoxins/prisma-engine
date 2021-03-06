use super::*;
use crate::common::ScalarType;
use crate::{dml, IndexType};
use serde_json;

pub fn render_to_dmmf(schema: &dml::Datamodel) -> String {
    let dmmf = schema_to_dmmf(schema);
    serde_json::to_string_pretty(&dmmf).expect("Failed to render JSON")
}

pub fn render_to_dmmf_value(schema: &dml::Datamodel) -> serde_json::Value {
    let dmmf = schema_to_dmmf(schema);
    serde_json::to_value(&dmmf).expect("Failed to render JSON")
}

fn schema_to_dmmf(schema: &dml::Datamodel) -> Datamodel {
    let mut datamodel = Datamodel {
        models: vec![],
        enums: vec![],
    };

    for enum_model in schema.enums() {
        datamodel.enums.push(enum_to_dmmf(&enum_model));
    }

    for model in schema.models() {
        datamodel.models.push(model_to_dmmf(&model));
    }

    datamodel
}

fn enum_to_dmmf(en: &dml::Enum) -> Enum {
    let mut enm = Enum {
        name: en.name.clone(),
        values: vec![],
        db_name: en.database_name.clone(),
        documentation: en.documentation.clone(),
    };

    for enum_value in &en.values {
        enm.values.push(enum_value_to_dmmf(enum_value));
    }

    enm
}

fn enum_value_to_dmmf(en: &dml::EnumValue) -> EnumValue {
    EnumValue {
        name: en.name.clone(),
        db_name: en.database_name.clone(),
    }
}

fn model_to_dmmf(model: &dml::Model) -> Model {
    Model {
        name: model.name.clone(),
        db_name: model.database_name.clone(),
        is_embedded: model.is_embedded,
        fields: model.fields().map(|f| field_to_dmmf(model, f)).collect(),
        is_generated: Some(model.is_generated),
        documentation: model.documentation.clone(),
        id_fields: model.id_fields.clone(),
        unique_fields: model
            .indices
            .iter()
            .filter_map(|i| {
                if i.tpe == IndexType::Unique {
                    Some(i.fields.clone())
                } else {
                    None
                }
            })
            .collect(),
    }
}

fn field_to_dmmf(model: &dml::Model, field: &dml::Field) -> Field {
    let a_relation_field_is_based_on_this_field: bool = model.fields.iter().any(|f| match &f.field_type {
        dml::FieldType::Relation(rel_info) => rel_info.fields.contains(&field.name),
        _ => false,
    });
    Field {
        name: field.name.clone(),
        kind: get_field_kind(field),
        is_required: field.arity == dml::FieldArity::Required,
        is_list: field.arity == dml::FieldArity::List,
        is_id: field.is_id,
        is_read_only: a_relation_field_is_based_on_this_field,
        has_default_value: field.default_value.is_some(),
        is_unique: field.is_unique,
        relation_name: get_relation_name(field),
        relation_from_fields: get_relation_from_fields(field),
        relation_to_fields: get_relation_to_fields(field),
        relation_on_delete: get_relation_delete_strategy(field),
        field_type: get_field_type(field),
        is_generated: Some(field.is_generated),
        is_updated_at: Some(field.is_updated_at),
        documentation: field.documentation.clone(),
    }
}

fn get_field_kind(field: &dml::Field) -> String {
    match field.field_type {
        dml::FieldType::Relation(_) => String::from("object"),
        dml::FieldType::Enum(_) => String::from("enum"),
        dml::FieldType::Base(_, _) => String::from("scalar"),
        _ => unimplemented!("DMMF does not support field type {:?}", field.field_type),
    }
}

fn get_field_type(field: &dml::Field) -> String {
    match &field.field_type {
        dml::FieldType::Relation(relation_info) => relation_info.to.clone(),
        dml::FieldType::Enum(t) => t.clone(),
        dml::FieldType::Unsupported(t) => t.clone(),
        dml::FieldType::Base(t, _) => type_to_string(t),
        dml::FieldType::ConnectorSpecific(sft) => type_to_string(&sft.prisma_type()),
    }
}

fn type_to_string(scalar: &ScalarType) -> String {
    scalar.to_string()
}

fn get_relation_name(field: &dml::Field) -> Option<String> {
    match &field.field_type {
        dml::FieldType::Relation(relation_info) => Some(relation_info.name.clone()),
        _ => None,
    }
}

fn get_relation_from_fields(field: &dml::Field) -> Option<Vec<String>> {
    match &field.field_type {
        dml::FieldType::Relation(relation_info) => Some(relation_info.fields.clone()),
        _ => None,
    }
}

fn get_relation_to_fields(field: &dml::Field) -> Option<Vec<String>> {
    match &field.field_type {
        dml::FieldType::Relation(relation_info) => Some(relation_info.to_fields.clone()),
        _ => None,
    }
}

fn get_relation_delete_strategy(field: &dml::Field) -> Option<String> {
    match &field.field_type {
        dml::FieldType::Relation(relation_info) => Some(relation_info.on_delete.to_string()),
        _ => None,
    }
}
