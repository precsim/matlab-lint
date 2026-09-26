function y = range_spacing(A)
  y = A(1:2:end);
  B = [1 : 5];
  z = A(:, :);
  y = y + z(1);
end
