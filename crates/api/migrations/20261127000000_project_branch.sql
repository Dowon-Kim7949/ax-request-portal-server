-- The branch a project's AI works from.
--
-- The report and the work run clone this branch, and the work run opens its
-- pull request into it. Null means the repository's default branch, which is
-- what every project used before. The name rules (git's, as the contract's
-- `BranchName`) are checked when it is saved; the column keeps only the
-- shape a check constraint can see.

alter table projects add column repository_branch text
    check (repository_branch is null or (repository_branch <> '' and repository_branch = btrim(repository_branch)
                                         and length(repository_branch) <= 200));
