function out = functions_and_control(x)
  out = zeros(size(x));
  for k = 1:numel(x)
    if x(k) > 0
      out(k) = x(k)^2;
    elseif x(k) < 0
      out(k) = -x(k);
    else
      out(k) = 0;
    end
  end
end
