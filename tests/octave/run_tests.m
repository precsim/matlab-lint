function run_tests
  assert(1 + 1 == 2);

  repo_root = fileparts(fileparts(fileparts(mfilename('fullpath'))));
  input_file = fullfile(repo_root, 'tests', 'fixtures', 'formatter', 'runtime', 'input.m');
  expected_file = fullfile(repo_root, 'tests', 'fixtures', 'formatter', 'runtime', 'expected.m');

  run(input_file);
  clear x y;
  run(expected_file);

  corpus_dir = fullfile(repo_root, 'tests', 'corpus');
  addpath(corpus_dir);
  assert(isequal(functions_and_control([2 -3 0]), [4 3 0]));
  assert(strcmp(switch_try(1), 'one'));
  assert(strcmp(switch_try(3), 'few'));
  [a, c] = matrix_cell();
  assert(isequal(a, [1 -2 3; 4 5 6]));
  assert(strcmp(c{1, 1}, 'alpha'));
  assert(continuation(1, 2, 3) == 6);
  assert(nested_function(4) == 16);
  rmpath(corpus_dir);

  disp('mstyle Octave formatter regression tests passed');
end
