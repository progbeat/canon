use crate::check::command::output::{
    write_caller_result_output, write_result_output_without_started_report,
};
use crate::check::core::CheckRecord;
use std::time::Duration;

pub(in crate::check::engine::execute::expectation) fn append_check_result_to_user_visible_report(
    started_report: super::super::super::progress::LiveExpectationReport,
    record: &CheckRecord,
) {
    started_report.append_result_before_structured_record(record);
}

pub(in crate::check::engine::execute::expectation) fn write_user_visible_caller_check_result(
    result_output: &mut Option<&mut dyn std::io::Write>,
    record: &CheckRecord,
    elapsed: Duration,
) -> Result<(), String> {
    write_caller_result_output(result_output, record, elapsed)
}

pub(in crate::check::engine::execute::expectation) fn write_user_visible_check_result_without_started_report(
    result_output: &mut Option<&mut dyn std::io::Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    write_result_output_without_started_report(result_output, record)
}
