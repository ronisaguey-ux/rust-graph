# Exceptions: begin/rescue/else/ensure.
def divide(a, b)
  begin
    result = a / b
  rescue ZeroDivisionError
    result = 0
  rescue StandardError
    result = nil
  ensure
    puts("done")
  end
  result
end

def risky
  begin
    divide(1, 0)
  rescue
    return nil
  else
    return 1
  end
end

puts(divide(6, 3))
puts(risky)
