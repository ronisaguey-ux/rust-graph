# Rules a scanner applies to a parsed file.
const DEFAULT_WEIGHT = 0.35

abstract type Rule end

struct SizeRule <: Rule
    max_bytes::Int
end

struct DepthRule <: Rule
    max_depth::Int
end

function applies(rule::SizeRule, file)
    return filesize(file) <= rule.max_bytes
end

function applies(rule::DepthRule, file)
    depth = length(split(file, '/'))
    return depth <= rule.max_depth
end

function evaluate(rules::Vector{Rule}, files::Vector{String})
    results = Dict{String, Vector{Bool}}()
    for file in files
        results[file] = [applies(rule, file) for rule in rules]
    end
    return results
end

const RULES = [SizeRule(1_000_000), DepthRule(8)]

passed(results) = all(v -> all(v), values(results))
