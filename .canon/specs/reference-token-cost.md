# Reference Token Cost

Reference token cost is a model-agnostic optimization metric.

It uses observed token counts and fixed reference prices per one million tokens.
The reference prices preserve a reasonable order of magnitude and relative
weights between uncached input, cached input, and output tokens.

```
def reference_token_cost(input_tokens: int, cached_input_tokens: int, output_tokens: int) -> float:
    UNCACHED_INPUT_1M_REFERENCE_PRICE = 1.0
    CACHED_INPUT_1M_REFERENCE_PRICE = 0.1
    OUTPUT_1M_REFERENCE_PRICE = 10.0
    uncached_input = input_tokens - cached_input_tokens
    assert uncached_input >= 0, "cached_input_tokens cannot exceed input_tokens"
    return (
        uncached_input * UNCACHED_INPUT_1M_REFERENCE_PRICE +
        cached_input_tokens * CACHED_INPUT_1M_REFERENCE_PRICE +
        output_tokens * OUTPUT_1M_REFERENCE_PRICE
    ) / 1_000_000
```
