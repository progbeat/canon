# After start, owners log newly known or changed evaluation data:
#   log.info('xpec.evaluation.update', id=xpec.id, ...)

log = import(ref="#log")


@ref("#evaluate")
def evaluate(xpec):
    evaluator_type = {
        CALLER: _CallerEvaluator,
        AGENT: _AgentEvaluator,
        SHELL: _ShellEvaluator,
    }.get(xpec.to)
    assert evaluator_type is not None, f"Unknown xpec.to: {xpec.to}"
    evaluator = evaluator_type(xpec)
    evaluator.before_start()
    log.info('xpec.evaluation.start', id=xpec.id, ...)
    with progress_timeline(xpec.id, ...) as timeline:
        try:
            response = evaluator.interrogate()
        except Exception:
            response = Response(error=...)  # `error` must identify the kind of failure
        except BaseException:
            response = Response(error=...)
            raise
        finally:
            timeline.stop()
            status = PASS if response.error is None and evaluator.check_answer(response.answer) else FAIL
            log.info('xpec.evaluation.finish', id=xpec.id, ...)
    return {"status": status, "response": response}


class _Evaluator:
    def __init__(self, xpec):
        self.xpec = xpec

    def check_answer(self, answer):
        return answer == self.xpec.a

    def before_start(self):
        pass


class _CallerEvaluator(_Evaluator):
    def before_start(self):
        if interactive_posix_terminal:
            print(_hide_from_human(self.xpec.q), end='', flush=True)
        else:
            print(escape_inline(self.xpec.q), end=' ', flush=True)

    def interrogate(self):
        return Response(answer=input())


class _AgentEvaluator(_Evaluator):
    def interrogate(self):
        initial_q_scope = resolve_q_scope(self.xpec)
        with interrogation_policy.start(self.xpec) as interrogation:
            # After a technical evaluator failure, including when an attempt exhausts its
            # no-progress timeout, `turn` applies any applicable retries of the current
            # model before trying later configured models in fallback order. The
            # interrogation fails if no attempt succeeds.
            response = interrogation.turn(q_scope=initial_q_scope)
            if ...:  # q-scope is auto and the evaluation may hide files from evaluator turns
                if response.error == "ScopeTooNarrow":
                    assert initial_q_scope != FULL_PROJECT_SCOPE, "ScopeTooNarrow error on full project scope"
                    response = interrogation.turn(q_scope=FULL_PROJECT_SCOPE)
                if self.check_answer(response.answer) and response.qScopeSuggestion is not None:
                    is_narrow_enough = ...  # whether the visible tree induced by the q-scope suggestion has at least 25% fewer files than the current visible tree
                    if is_narrow_enough:
                        follow_up_response = interrogation.turn(q_scope=response.qScopeSuggestion)
                        if follow_up_response.answer is not None:
                            response = follow_up_response
            assert len(interrogation.turns) <= 3, "unexpectedly many turns in interrogation"
        return response


class _ShellEvaluator(_Evaluator):
    def interrogate(self):
        transcript = StringIO()
        transcript.write(f'$ {self.xpec.q}\n')
        exit_code = shell.run(self.xpec.q, stdin=CLOSED, stdout=transcript, stderr=transcript)
        return Response(
            answer=str(exit_code),
            evidence=transcript.getvalue(),
        )


def _hide_from_human(text):
    one_line_text = text.replace('\n', '\r').strip()
    return f'{one_line_text}\r{CSI_ERASE_LINE}'
