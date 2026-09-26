function y = switch_try(x)
  try
    switch x
      case 1
        y = 'one';
      case {2, 3}
        y = 'few';
      otherwise
        y = 'other';
    end
  catch err
    y = err.message;
  end
end
