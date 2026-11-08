#!/usr/bin/env julia

using Pkg
using Statistics

function describe(values::Vector{Float64})::Dict{String, Float64}
    if isempty(values)
        return Dict("n" => 0.0, "mean" => 0.0)
    end
    return Dict(
        "n" => float(length(values)),
        "mean" => mean(values),
        "max" => maximum(values),
    )
end

function main(args)
    if length(args) < 1
        println("usage: pipeline.jl <file>")
        return 1
    end
    path = args[1]
    numbers = Float64[]
    for line in eachline(path)
        value = tryparse(Float64, line)
        if value !== nothing
            push!(numbers, value)
        end
    end
    summary = describe(numbers)
    println(summary)
    return 0
end

exit(main(ARGS))
