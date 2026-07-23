@ref("#emit_check_feedback")
def emit_check_feedback(failed, num_pending):
    """
    :param failed: Short IDs of failed expectations.
    :param num_pending: Number of pending expectations.
    """
    assert against_tree_oid == head_tree_oid
    if len(failed) > 0:
        _repair_instructions(failed)
        return print(f"▷ Fix the issues and run `canon check` again!")
    if num_pending > 0:
        return print("▷ Run `canon check` to continue evaluation.")
    need_to_commit = (checked_tree_oid != against_tree_oid)
    print(
        "✓ All checks passed." +
        (" Commit the staged changes!" if need_to_commit else "")
    )


def _repair_instructions(failed):
    assert len(failed) > 0
    print("❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.")
    if len(failed) == 1:
        # `fail_history` is a global chronological (bounded) history of failure records across `canon check` runs.
        assert fail_history[-1].xpec.short_id == failed[0], "the current failure record must be appended to `fail_history`"
        if _emit_recurring_xpec_failures_warning():
            return
    # These failures were already shown in `canon check` output, so don't show them again to save tokens.
    selectors = ' '.join(f'not:{x}' for x in failed)
    print(f"❕ Plan the repair, then run `canon show {selectors} -- <PATHSPEC>...` for the planned edit paths to identify expectations that may be affected.")
    print("❕ Use the matching expectations to avoid regressions while fixing the issues.")


def _emit_recurring_xpec_failures_warning():
    TAIL = 2
    last_short_ids = []
    for failure in reversed(fail_history):
        if (
            failure.head_tree_oid != head_tree_oid
            or failure.xpec.to != AGENT
            or failure.response.error is not None
        ):
            if len(last_short_ids) == 0:
                return False
            continue
        last_short_ids.append(failure.xpec.short_id)
        if len(last_short_ids) == TAIL:
            break
    if len(last_short_ids) != TAIL or len(set(last_short_ids)) != 1:
        return False
    print("❕ Repeated `canon check` runs keep failing on the same xpec. Do not run `canon check` again yet.")
    print("❕ Each time this warning appears, determine why your workflow allowed the recurrence and adapt it to reduce the chance of another one.")
    print("❕ Emulate the evaluator agent: independently try to disprove the expected answer. Generalize each finding and fix every supported violation.")
    failed_xpec = fail_history[-1].xpec
    if failed_xpec.target == "diff":
        print(
            f"❕ Xpec `{failed_xpec.short_id}` targets the diff. Look for violations only "
            f"in files listed by `git diff --cached --numstat {failed_xpec.diff_from_oid}`."
        )
    print("▷ Run `canon check` again only after you can independently justify the expected answer!")
    return True
