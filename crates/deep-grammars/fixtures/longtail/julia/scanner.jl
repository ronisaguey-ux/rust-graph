module Scanner

using Dates
using JSON

export scan, Summary

struct Summary
    count::Int
    bytes::Int
    elapsed::Float64
end

mutable struct State
    root::String
    depth::Int
    seen::Dict{String, Bool}
end

function state(root::String; depth::Int = 12)
    return State(root, depth, Dict{String, Bool}())
end

function walk(state::State, dir::String = state.root)
    entries = readdir(dir; join = true)
    files = String[]
    for entry in entries
        if isdir(entry)
            append!(files, walk(state, entry))
        elseif !haskey(state.seen, entry)
            state.seen[entry] = true
            push!(files, entry)
        end
    end
    return files
end

function scan(root::String; depth::Int = 12)
    started = time()
    state = Scanner.state(root, depth = depth)
    files = walk(state)
    bytes = sum(filesize(f) for f in files)
    return Summary(length(files), bytes, time() - started)
end

end
