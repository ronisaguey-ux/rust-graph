require "json"

module Scanner
  enum Kind
    Password
    Secret
    Token
  end

  struct Rule
    getter pattern : String
    getter kind : Kind

    def initialize(@pattern : String, @kind : Kind)
    end
  end

  def self.match?(rule : Rule, text : String) : Bool
    if text.empty?
      false
    else
      text.includes?(rule.pattern)
    end
  end

  def self.scan(rules : Array(Rule), text : String) : Int32
    hits = 0
    rules.each do |rule|
      case rule.kind
      when Kind::Password
        if match?(rule, text)
          hits += 1
        end
      when Kind::Secret, Kind::Token
        hits += 2
      else
        next
      end
    end
    hits
  end
end
