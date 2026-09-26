function y = nested_function(x)
  y = helper(x);

  function z = helper(v)
    z = v .* v;
  end
end
