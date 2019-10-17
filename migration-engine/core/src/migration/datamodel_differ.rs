#![deny(rust_2018_idioms)]

mod directives;
mod enums;
mod fields;
mod models;
mod top_level;

use directives::DirectiveDiffer;
use enums::EnumDiffer;
use fields::FieldDiffer;
use models::ModelDiffer;
use top_level::TopDiffer;

use datamodel::ast;
use migration_connector::steps::{self, MigrationStep};

/// Diff two datamodels, returning the [MigrationStep](/struct.MigrationStep.html)s from `previous`
/// to `next`.
pub(crate) fn diff(previous: &ast::SchemaAst, next: &ast::SchemaAst) -> Vec<MigrationStep> {
    let mut steps = Vec::new();
    let differ = TopDiffer { previous, next };

    push_enums(&mut steps, &differ);
    push_models(&mut steps, &differ);

    steps
}

type Steps = Vec<MigrationStep>;

fn push_enums(steps: &mut Steps, differ: &TopDiffer<'_>) {
    push_created_enums(steps, differ.created_enums());
    push_deleted_enums(steps, differ.deleted_enums());
    push_updated_enums(steps, differ.enum_pairs());
}

fn push_created_enums<'a>(steps: &mut Steps, enums: impl Iterator<Item = &'a ast::Enum>) {
    for r#enum in enums {
        let create_enum_step = steps::CreateEnum {
            name: r#enum.name.name.clone(),
            values: r#enum.values.iter().map(|value| value.name.clone()).collect(),
        };

        steps.push(MigrationStep::CreateEnum(create_enum_step));

        let location = steps::DirectiveLocation::Enum {
            r#enum: r#enum.name.name.clone(),
        };

        push_created_directives(steps, &location, r#enum.directives.iter());
    }
}

fn push_deleted_enums<'a>(steps: &mut Steps, enums: impl Iterator<Item = &'a ast::Enum>) {
    let deleted_enum_steps = enums
        .map(|deleted_enum| steps::DeleteEnum {
            name: deleted_enum.name.name.clone(),
        })
        .map(MigrationStep::DeleteEnum);

    steps.extend(deleted_enum_steps)
}

fn push_updated_enums<'a>(steps: &mut Steps, enums: impl Iterator<Item = EnumDiffer<'a>>) {
    for updated_enum in enums {
        let created_values: Vec<_> = updated_enum
            .created_values()
            .map(|value| value.name.to_owned())
            .collect();
        let deleted_values: Vec<_> = updated_enum
            .deleted_values()
            .map(|value| value.name.to_owned())
            .collect();

        let update_enum_step = steps::UpdateEnum {
            name: updated_enum.previous.name.name.clone(),
            new_name: diff_value(&updated_enum.previous.name.name, &updated_enum.next.name.name),
            created_values,
            deleted_values,
        };

        if update_enum_step.is_any_option_set() {
            steps.push(MigrationStep::UpdateEnum(update_enum_step));
        }

        let location = steps::DirectiveLocation::Enum {
            r#enum: updated_enum.previous.name.name.clone(),
        };

        push_created_directives(steps, &location, updated_enum.created_directives());
        push_updated_directives(steps, &location, updated_enum.directive_pairs());
        push_deleted_directives(steps, &location, updated_enum.deleted_directives());
    }
}

fn push_models(steps: &mut Steps, differ: &TopDiffer<'_>) {
    push_created_models(steps, differ.created_models());
    push_deleted_models(steps, differ.deleted_models());
    push_updated_models(steps, differ.model_pairs());
}

fn push_created_models<'a>(steps: &mut Steps, models: impl Iterator<Item = &'a ast::Model>) {
    for created_model in models {
        let directive_location = steps::DirectiveLocation::Model {
            model: created_model.name.name.clone(),
        };

        let db_name =
            directives::get_directive_string_value("map", &created_model.directives).map(|db_name| db_name.to_owned());

        let create_model_step = steps::CreateModel {
            name: created_model.name.name.clone(),
            embedded: false, // not represented in the AST yet
            db_name,
        };

        steps.push(MigrationStep::CreateModel(create_model_step));

        push_created_fields(steps, &created_model.name.name, created_model.fields.iter());
        push_created_directives(steps, &directive_location, created_model.directives.iter());
    }
}

fn push_deleted_models<'a>(steps: &mut Steps, models: impl Iterator<Item = &'a ast::Model>) {
    let delete_model_steps = models
        .map(|deleted_model| steps::DeleteModel {
            name: deleted_model.name.name.clone(),
        })
        .map(MigrationStep::DeleteModel);

    steps.extend(delete_model_steps);
}

fn push_updated_models<'a>(steps: &mut Steps, models: impl Iterator<Item = ModelDiffer<'a>>) {
    models.for_each(|model| {
        let model_name = &model.previous.name.name;

        push_created_fields(steps, model_name, model.created_fields());
        push_deleted_fields(steps, model_name, model.deleted_fields());
        push_updated_fields(steps, model_name, model.field_pairs());

        let directive_location = steps::DirectiveLocation::Model {
            model: model_name.clone(),
        };

        push_created_directives(steps, &directive_location, model.created_directives());
        push_updated_directives(steps, &directive_location, model.directive_pairs());
        push_deleted_directives(steps, &directive_location, model.deleted_directives());
    });
}

fn push_created_fields<'a>(steps: &mut Steps, model_name: &'a str, fields: impl Iterator<Item = &'a ast::Field>) {
    for field in fields {
        let default = field
            .directives
            .iter()
            .find(|directive| directive.name.name == "default")
            .and_then(|directive| directive.arguments.get(0))
            .map(|argument| steps::MigrationExpression::from_ast_expression(&argument.value));

        let create_field_step = steps::CreateField {
            arity: field.arity.clone(),
            name: field.name.name.clone(),
            tpe: field.field_type.name.clone(),
            model: model_name.to_owned(),
            db_name: directives::get_directive_string_value("map", &field.directives).map(String::from),
            default,
        };

        steps.push(MigrationStep::CreateField(create_field_step));

        let directive_location = steps::DirectiveLocation::Field {
            model: model_name.to_owned(),
            field: field.name.name.clone(),
        };

        push_created_directives(steps, &directive_location, field.directives.iter());
    }
}

fn push_deleted_fields<'a>(steps: &mut Steps, model_name: &'a str, fields: impl Iterator<Item = &'a ast::Field>) {
    let delete_field_steps = fields
        .map(|deleted_field| steps::DeleteField {
            model: model_name.to_owned(),
            name: deleted_field.name.name.clone(),
        })
        .map(MigrationStep::DeleteField);

    steps.extend(delete_field_steps);
}

fn push_updated_fields<'a>(steps: &mut Steps, model_name: &'a str, fields: impl Iterator<Item = FieldDiffer<'a>>) {
    for field in fields {
        let previous_default_directive = field
            .previous
            .directives
            .iter()
            .find(|directive| directive.name.name == "default")
            .and_then(|directive| directive.arguments.get(0))
            .map(|argument| steps::MigrationExpression::from_ast_expression(&argument.value));

        let next_default_directive = field
            .next
            .directives
            .iter()
            .find(|directive| directive.name.name == "default")
            .and_then(|directive| directive.arguments.get(0))
            .map(|argument| steps::MigrationExpression::from_ast_expression(&argument.value));

        let update_field_step = steps::UpdateField {
            arity: diff_value(&field.previous.arity, &field.next.arity),
            new_name: diff_value(&field.previous.name.name, &field.next.name.name),
            model: model_name.to_owned(),
            name: field.previous.name.name.clone(),
            tpe: diff_value(&field.previous.field_type.name, &field.next.field_type.name),
            default: diff_value(&previous_default_directive, &next_default_directive),
        };

        if update_field_step.is_any_option_set() {
            steps.push(MigrationStep::UpdateField(update_field_step));
        }

        let directive_location = steps::DirectiveLocation::Field {
            model: model_name.to_owned(),
            field: field.previous.name.name.clone(),
        };

        push_created_directives(steps, &directive_location, field.created_directives());
        push_updated_directives(steps, &directive_location, field.directive_pairs());
        push_deleted_directives(steps, &directive_location, field.deleted_directives());
    }
}

fn push_created_directives<'a>(
    steps: &mut Steps,
    location: &steps::DirectiveLocation,
    directives: impl Iterator<Item = &'a ast::Directive>,
) {
    for directive in directives {
        push_created_directive(steps, location.clone(), directive);
    }
}

fn push_created_directive(steps: &mut Steps, location: steps::DirectiveLocation, directive: &ast::Directive) {
    let locator = steps::DirectiveLocator {
        location,
        name: directive.name.name.clone(),
    };

    let step = steps::CreateDirective {
        locator: locator.clone(),
    };

    steps.push(MigrationStep::CreateDirective(step));

    for argument in &directive.arguments {
        push_created_directive_argument(steps, &locator, argument);
    }
}

fn push_deleted_directives<'a>(
    steps: &mut Steps,
    location: &steps::DirectiveLocation,
    directives: impl Iterator<Item = &'a ast::Directive>,
) {
    for directive in directives {
        push_deleted_directive(steps, location.clone(), directive);
    }
}

fn push_deleted_directive(steps: &mut Steps, location: steps::DirectiveLocation, directive: &ast::Directive) {
    let step = steps::DeleteDirective {
        locator: steps::DirectiveLocator {
            location,
            name: directive.name.name.clone(),
        },
    };

    steps.push(MigrationStep::DeleteDirective(step));
}

fn push_updated_directives<'a>(
    steps: &mut Steps,
    location: &steps::DirectiveLocation,
    directives: impl Iterator<Item = DirectiveDiffer<'a>>,
) {
    for directive in directives {
        push_updated_directive(steps, location.clone(), directive);
    }
}

fn push_updated_directive(steps: &mut Steps, location: steps::DirectiveLocation, directive: DirectiveDiffer<'_>) {
    let locator = steps::DirectiveLocator {
        name: directive.previous.name.name.clone(),
        location: location.clone(),
    };

    for argument in directive.created_arguments() {
        push_created_directive_argument(steps, &locator, &argument);
    }

    for (previous, next) in directive.argument_pairs() {
        push_updated_directive_argument(steps, &locator, previous, next);
    }

    for argument in directive.deleted_arguments() {
        push_deleted_directive_argument(steps, &locator, &argument.name.name);
    }
}

fn push_created_directive_argument(
    steps: &mut Steps,
    directive_location: &steps::DirectiveLocator,
    argument: &ast::Argument,
) {
    let create_argument_step = steps::CreateDirectiveArgument {
        argument_name: argument.name.name.clone(),
        argument_value: steps::MigrationExpression::from_ast_expression(&argument.value),
        directive_location: directive_location.clone(),
    };

    steps.push(MigrationStep::CreateDirectiveArgument(create_argument_step));
}

fn push_updated_directive_argument(
    steps: &mut Steps,
    directive_location: &steps::DirectiveLocator,
    previous_argument: &ast::Argument,
    next_argument: &ast::Argument,
) {
    let previous_value = steps::MigrationExpression::from_ast_expression(&previous_argument.value);
    let next_value = steps::MigrationExpression::from_ast_expression(&next_argument.value);

    if previous_value == next_value {
        return;
    }

    let update_argument_step = steps::UpdateDirectiveArgument {
        argument_name: next_argument.name.name.clone(),
        new_argument_value: next_value,
        directive_location: directive_location.clone(),
    };

    steps.push(MigrationStep::UpdateDirectiveArgument(update_argument_step));
}

fn push_deleted_directive_argument(
    steps: &mut Steps,
    directive_location: &steps::DirectiveLocator,
    argument_name: &str,
) {
    let delete_argument_step = steps::DeleteDirectiveArgument {
        argument_name: argument_name.to_owned(),
        directive_location: directive_location.clone(),
    };

    steps.push(MigrationStep::DeleteDirectiveArgument(delete_argument_step));
}

fn diff_value<T: PartialEq + Clone>(current: &T, updated: &T) -> Option<T> {
    if current == updated {
        None
    } else {
        Some(updated.clone())
    }
}
